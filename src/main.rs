#[macro_use]
extern crate log;
#[macro_use]
extern crate quick_error;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;

use config::{get_config, init_config};
use futures::prelude::*;
use hyper::{service::make_service_fn, Server as HttpServer};
use ring::rand::{SecureRandom, SystemRandom};
use services::auth::SharedSecretAuthenticator;
use services::search::Search;
use services::{FileSendService, TranscodingDetails};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::pin::Pin;
use std::process;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

mod config;
mod error;
mod services;
mod util;
#[cfg(feature = "tls")]
mod tls;

fn generate_server_secret<P: AsRef<Path>>(file: P) -> Result<Vec<u8>, io::Error> {
    let file = file.as_ref();
    if file.exists() {
        let mut v = vec![];
        let size = file.metadata()?.len();
        if size > 128 {
            return Err(io::Error::new(io::ErrorKind::Other, "Secret too long"));
        }

        let mut f = File::open(file)?;
        f.read_to_end(&mut v)?;
        Ok(v)
    } else {
        let mut random = [0u8; 32];
        let rng = SystemRandom::new();
        rng.fill(&mut random)
            .map_err(|_e| io::Error::new(io::ErrorKind::Other, "Error when generating secret"))?;
        let mut f;
        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            use std::os::unix::fs::OpenOptionsExt;
            f = OpenOptions::new()
                .mode(0o600)
                .create(true)
                .write(true)
                .truncate(true)
                .open(file)?
        }
        #[cfg(not(unix))]
        {
            f = File::create(file)?
        }
        f.write_all(&random)?;
        Ok(random.to_vec())
    }
}

type RuntimeError = Box<dyn std::error::Error+'static>;

fn start_server(
    server_secret: Vec<u8>,
) -> Result<tokio::runtime::Runtime, RuntimeError> {
    let cfg = get_config();
    let svc = FileSendService {
        authenticator: get_config().shared_secret.as_ref().map(
            |secret| -> Arc<Box<dyn services::auth::Authenticator<Credentials = ()>>> {
                Arc::new(Box::new(SharedSecretAuthenticator::new(
                    secret.clone(),
                    server_secret,
                    cfg.token_validity_hours,
                )))
            },
        ),
        search: Search::new(),
        transcoding: TranscodingDetails {
            transcodings: Arc::new(AtomicUsize::new(0)),
            max_transcodings: cfg.transcoding.max_parallel_processes,
        },
    };
    let addr = cfg.listen;
    let start_server = async move {
        

        let server: Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send>> =
            match get_config().ssl.as_ref() {
                None => {
                    let server = HttpServer::bind(&addr).serve(make_service_fn(
                        move |_| future::ok::<_, error::Error>(svc.clone()),
                    ));
                    info!("Server listening on {}", &addr);
                    Box::pin(server.map_err(|e| e.into()))
                }
                Some(ssl) => {
                    #[cfg(feature = "tls")]
                    {   
                        info!("Server Listening on {} with TLS", &addr);
                        let create_server = async move {
                            let incoming = tls::tls_acceptor(&addr,&ssl).await?;
                            let server = HttpServer::builder(incoming).serve(make_service_fn(
                                move |_| future::ok::<_, error::Error>(svc.clone()),
                            )).await;
                            
                            server.map_err(|e| e.into())
                        };
                        

                        
                        Box::pin(create_server)
                    }

                    #[cfg(not(feature = "tls"))]
                    {
                        panic!(
                            "TLS is not compiled - build with default features {:?}",
                            ssl
                        )
                    }
                }
            };

        server.await
    };

    let rt = tokio::runtime::Builder::new()
        .threaded_scheduler()
        .enable_all()
        .core_threads(cfg.thread_pool.num_threads as usize)
        .max_threads(cfg.thread_pool.num_threads as usize + cfg.thread_pool.queue_size as usize)
        .build()
        .unwrap();

    rt.spawn(start_server.map_err(|e| error!("Http Server Error: {}", e)));
    Ok(rt)
}

#[cfg(not(unix))]
async fn terminate_server() {
    use tokio::signal;
    signal::ctrl_c().await.unwrap_or(());

}

#[cfg(unix)]
async fn terminate_server() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt()).expect("Cannot create SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("Cannot create SIGTERM handler");
    let mut sigquit = signal(SignalKind::quit()).expect("Cannot create SIGQUIT handler");

    tokio::select!(
        _ = sigint.next() => {info!("Terminated on SIGINT")},
        _ = sigterm.next() => {info!("Terminated on SIGTERM")},
        _ = sigquit.next() => {info!("Terminated on SIGQUIT")}
    )

}

fn main() {
    #[cfg(unix)]
    {
        if nix::unistd::getuid().is_root() {
            warn!("Audioserve is running as root! Not recommended.")
        }
    }
    if let Err(e) = init_config() {
        eprintln!("Config/Arguments error: {}", e);
        process::exit(1)
    };
    env_logger::init();
    debug!("Started with following config {:?}", get_config());

    services::audio_meta::init_media_lib();

    #[cfg(feature = "transcoding-cache")]
    {
        use crate::services::transcode::cache::get_cache;
        if get_config().transcoding.cache.disabled {
            info!("Trascoding cache is disabled")
        } else {
            let c = get_cache();
            info!(
                "Using transcoding cache at {:?}, remaining capacity (files,size) : {:?}",
                get_config().transcoding.cache.root_dir,
                c.free_capacity()
            )
        }
    }
    let server_secret = match generate_server_secret(&get_config().secret_file) {
        Ok(s) => s,
        Err(e) => {
            error!("Error creating/reading secret: {}", e);
            process::exit(2)
        }
    };

    let mut runtime = match start_server(server_secret) {
        Ok(rt) => rt,
        Err(e) => {
            error!("Error starting server: {}", e);
            process::exit(3)
        }
    };

    runtime.block_on(terminate_server());

    #[cfg(feature = "shared-positions")]{
        debug!("Saving shared positions");
        runtime.block_on(crate::services::position::save_positions());
    }
        //graceful shutdown of server will wait till transcoding ends, so rather shut it down hard
    runtime.shutdown_timeout(std::time::Duration::from_millis(300));

    #[cfg(feature = "transcoding-cache")]
    {
        debug!("Saving transcoding cache");
        use crate::services::transcode::cache::get_cache;
        if let Err(e) = get_cache().save_index_blocking() {
            error!("Error saving transcoding cache index {}", e);
        }
    }
        
    
    info!("Server finished");
}

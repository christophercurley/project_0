use daymark_api::{App, Config};
use std::{io::IsTerminal, net::SocketAddr};
use zeroize::Zeroizing;

fn main() {
    tracing_subscriber::fmt().json().with_target(false).init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .max_blocking_threads(4)
        .build()
        .expect("runtime initialization");
    if runtime.block_on(run()).is_err() {
        // Errors never print CLI input, database contents, credentials or hashes.
        tracing::error!(
            event = "application_failed",
            message =
                "Check configuration, database access and whether another instance holds the lock."
        );
        std::process::exit(1);
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path, username] if command == "bootstrap" => {
            if !std::io::stdin().is_terminal() {
                return Err("interactive terminal required".into());
            }
            let password = Zeroizing::new(rpassword::prompt_password(
                "Initial administrator password (12–128 bytes): ",
            )?);
            let repeated = Zeroizing::new(rpassword::prompt_password("Repeat password: ")?);
            if *password != *repeated {
                return Err("passwords differ".into());
            }
            let app = App::open(path, Config::https("https://localhost")?).await?;
            app.bootstrap(username.clone(), password.to_string())
                .await?;
            tracing::info!(event = "administrator_bootstrapped");
        }
        [command, path, origin, bind, rest @ ..] if command == "serve" => {
            let local = rest == ["--local"];
            if !rest.is_empty() && !local {
                return Err("invalid arguments".into());
            }
            let bind: SocketAddr = bind.parse()?;
            if local && !bind.ip().is_loopback() {
                return Err("local mode requires loopback bind".into());
            }
            let config = if local {
                Config::local(origin)?
            } else {
                Config::https(origin)?
            };
            let app = App::open(path, config).await?;
            let listener = tokio::net::TcpListener::bind(bind).await?;
            tracing::info!(event = "server_started");
            axum::serve(
                listener,
                app.router()
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        }
        _ => {
            eprintln!(
                "Usage: daymark-api bootstrap DATABASE USERNAME\n       daymark-api serve DATABASE ORIGIN BIND [--local]"
            );
            return Err("invalid arguments".into());
        }
    }
    Ok(())
}

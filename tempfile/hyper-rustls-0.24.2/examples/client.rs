//! Simple HTTPS GET client based on hyper-rustls
//!
//! First parameter is the mandatory URL to GET.
//! Second parameter is an optional path to CA store.
use hyper::{body::to_bytes, client, Body, Uri};
use hyper_rustls::ConfigBuilderExt;
use rustls::RootCertStore;

use std::str::FromStr;
use std::{env, fs, io};

fn main() {
    // Send GET request and inspect result, with proper error handling.
    if let Err(e) = run_client() {
        eprintln!("FAILED: {}", e);
        std::process::exit(1);
    }
}

fn error(err: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

#[tokio::main]
async fn run_client() -> io::Result<()> {
    // First parameter is target URL (mandatory).
    let url = match env::args().nth(1) {
        Some(ref url) => Uri::from_str(url).map_err(|e| error(format!("{}", e)))?,
        None => {
            println!("Usage: client <url> <ca_store>");
            return Ok(());
        }
    };

    // Second parameter is custom Root-CA store (optional, defaults to native cert store).
    let mut ca = match env::args().nth(2) {
        Some(ref path) => {
            let f = fs::File::open(path)
                .map_err(|e| error(format!("failed to open {}: {}", path, e)))?;
            let rd = io::BufReader::new(f);
            Some(rd)
        }
        None => None,
    };

    // Prepare the TLS client config
    let tls = match ca {
        Some(ref mut rd) => {
            // Read trust roots
            let certs = rustls_pemfile::certs(rd)
                .map_err(|_| error("failed to load custom CA store".into()))?;
            let mut roots = RootCertStore::empty();
            roots.add_parsable_certificates(&certs);
            // TLS client config using the custom CA store for lookups
            rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
        // Default TLS client config with native roots
        None => rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_native_roots()
            .with_no_client_auth(),
    };
    // Prepare the HTTPS connector
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http()
        .enable_http1()
        .build();

    // Build the hyper client from the HTTPS connector.
    let client: client::Client<_, hyper::Body> = client::Client::builder().build(https);

    // Prepare a chain of futures which sends a GET request, inspects
    // the returned headers, collects the whole body and prints it to
    // stdout.
    let fut = async move {
        let res = client
            .get(url)
            .await
            .map_err(|e| error(format!("Could not get: {:?}", e)))?;
        println!("Status:\n{}", res.status());
        println!("Headers:\n{:#?}", res.headers());

        let body: Body = res.into_body();
        let body = to_bytes(body)
            .await
            .map_err(|e| error(format!("Could not get body: {:?}", e)))?;
        println!("Body:\n{}", String::from_utf8_lossy(&body));

        Ok(())
    };

    fut.await
}

#![deny(warnings)]
#![warn(rust_2018_idioms)]
use std::io::{Error, stdin};
use std::thread::sleep;
use std::time::Duration;
use bytes::Bytes;

use hyper::{Request, Version};
use tokio::io::{self, AsyncWriteExt as _};
use tokio::net::TcpStream;

//use tokio_serial::SerialPortBuilderExt;

use http_body_util::{BodyExt, Empty};
use tokio::runtime::Builder;
use tokio::task::JoinHandle;


mod io_utils;
use io_utils::TokioIo;

type ResultHere<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;



fn main() -> ResultHere<()> {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("my-process")
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();

    let url = "http://caster.centipede.fr:2101/GDCRT";
    let mut ntrip_flux: JoinHandle<ResultHere<()>> = runtime.spawn(fetch_url(url));

    let is_exit: JoinHandle<ResultHere<()>> = runtime.spawn(check_if_exit());


    //let mut port = tokio_serial::new("COM3", 115200).open_native_async();

    loop{
        sleep(Duration::from_secs(1));
        if ntrip_flux.is_finished(){
            ntrip_flux = runtime.spawn(fetch_url(url));
        }
        if is_exit.is_finished(){
            break;
        }
    }

    runtime.shutdown_background();

    Ok(())
}

async fn check_if_exit() -> ResultHere<()> {
    let mut buffer = String::new();
    stdin().read_line(&mut buffer).unwrap();
    Ok(())
}


async fn fetch_url(url: &str) -> ResultHere<()> {
    let url = url.parse::<hyper::Uri>().unwrap();
    if url.scheme_str() != Some("http") {
        return Err(Box::new(Error::new(std::io::ErrorKind::InvalidInput, "This example only works with 'http' URLs.")));
    }


    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(addr).await?;
    let io = TokioIo::new(stream);


    let (mut sender, conn) = hyper::client::conn::http1::Builder::new().http09_responses(true).handshake(io).await?;

    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            println!("Connection failed: {:?}", err);
        }
    });

    let authority = url.authority().unwrap().clone();

    // curl -v --http0.9  http://caster.centipede.fr:2101/LESTR
    //   -H "User-Agent: NTRIP TnlAgClient/1.0"
    //   -H "Authorization: Basic Y2VudGlwZWRlOmNlbnRpcGVkZQ=="  --output -

    let req = Request::builder()
        .uri(url)
        .header(hyper::header::HOST, authority.as_str())
        //.header("User-Agent", "NTRIP TnlAgClient/1.0")
        .version(Version::HTTP_11)
        .body(Empty::<Bytes>::new())?;

    let mut res = sender.send_request(req).await?;

    println!("Response: {}", res.status());
    if !res.status().is_success(){
        return Err(Box::new(Error::new(std::io::ErrorKind::NotConnected, "The response is not success !")));
    }
    //println!("Headers: {:#?}\n", res.headers());

    // Stream the body, writing each chunk to stdout as we get it
    // (instead of buffering and printing at the end).
    while let Some(next) = res.frame().await {
        let frame = next?;
        if let Some(chunk) = frame.data_ref() {
            io::stdout().write_all(&chunk).await?;
        }
    }

    println!("Ntrip server done !");

    Ok(())
}
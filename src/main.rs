#![deny(warnings)]
#![warn(rust_2018_idioms)]
use std::io::{Error};
use bytes::Bytes;
use hyper::{Request, Response, Version};
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use tokio::io::{AsyncReadExt, AsyncWriteExt, WriteHalf};
use tokio::net::{TcpListener, TcpStream};

use tokio_serial::{SerialPortBuilderExt, SerialStream};

mod io_utils;
mod app_utils;

//use tokio_serial::SerialPortBuilderExt;
use crate::app_utils::ResultHere;
use crate::io_utils::TokioIo;

#[tokio::main]
async fn main() -> ResultHere<()> {
    let url = "http://caster.centipede.fr:2101/GDCRT";

    #[cfg(unix)]
    const COM: &str = "/dev/ttyACM0";
    #[cfg(windows)]
    const COM: &str = "COM3";

    println!("Will connect to {} and {}", url, COM);

    let ntrip_fix = ntrip_fix(url).await?;

    let port = tokio_serial::new(COM, 115200).open_native_async().unwrap();
    let (mut port_reader, port_writer) = tokio::io::split(port);

    let ntrip_handle=tokio::spawn(copy_ntrip_data(ntrip_fix, port_writer));

    let listener = TcpListener::bind("0.0.0.0:6543").await?;

    let (mut socket, _) = listener.accept().await?;
    println!("New client connected");

    loop {
        let mut buffer = [0; 10 * 1024];
        let read_result =  port_reader.read(&mut buffer).await;
        if let Ok(len) = read_result {
            let s=String::from_utf8(buffer[0..len].to_vec()).unwrap();
            let index = s.find("$GNGGA");
            if let Some(start)=index {
                match s.chars().skip(start).collect::<String>().find("\r") {
                    Some(end) => println!("Fix <- {}", s.chars().skip(start).take(end).collect::<String>()),
                    None => println!("Fix <- {}", s.chars().skip(start).collect::<String>()),
                }
            }

            if len > 0 {
                //println!("Read {} bytes", String::from_utf8(buffer[0..len].to_vec()).unwrap());
                if let Err(_)=socket.write_all(&buffer[0..len]).await {
                    break;
                }
            }
        }
    }

    ntrip_handle.await??;

    return Ok(());
}

async fn copy_ntrip_data(mut ntrip_fix: Response<Incoming>, mut port: WriteHalf<SerialStream>) -> ResultHere<()> {
        while let Some(next) = ntrip_fix.frame().await {
            let mut frame = next?;

            #[allow(unused_mut)]
            if let Some(chunk) = frame.data_mut() {
                println!("Send correction {} bytes", chunk.len());
                port.write_all(&chunk).await?;
            }
        }
        Ok(()) as ResultHere<()>
}

async fn ntrip_fix(url: &str) -> ResultHere<Response<Incoming>> {
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
        .header("User-Agent", "NTRIP GpsCorrect/1.0")
        .version(Version::HTTP_11)
        .body(Empty::<Bytes>::new())?;

    let mut res = sender.send_request(req).await?;

    if !res.status().is_success() {
        return Err(Box::new(Error::new(std::io::ErrorKind::NotConnected, "The response is not success !")));
    }

    if let Some(next) = res.frame().await {
        let mut frame = next?;

        #[allow(unused_mut)]
        if let Some(chunk) = frame.data_mut() {
            println!("---------------");
            // Check the first string is "ICY 200 OK\n\\n" and remove it
            let chunk_str = String::from_utf8(chunk[0..14].to_vec()).unwrap();
            if chunk_str != "ICY 200 OK\r\n\r\n" {
                return Err(Box::new(Error::new(std::io::ErrorKind::NotConnected, "The response is not success !")));
            } else {
                println!("Ntrip server initialized !")
            }
        }
    }

    println!("Ntrip server ok !");

    Ok(res)
}

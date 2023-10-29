#![deny(warnings)]
#![warn(rust_2018_idioms)]

use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Button, Grid};


use std::io::{Error};
use bytes::Bytes;
use gtk::glib::ExitCode;
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
async fn main() -> ExitCode {
    let app = Application::builder().application_id("GPS correct").build();

    // Connect to "activate" signal of `app`
    app.connect_activate(build_ui);

    // Run the application
    return app.run();
}

async fn start_process(url: String, com_port: String) -> ResultHere<()> {
    let com = match cfg!(unix) {
        true => "/dev/".to_owned()+com_port.as_str(),
        false => com_port.to_string(),
    };

    println!("Will connect to {} and {}", url, com);

    let ntrip_fix = ntrip_fix(url.as_str()).await?;

    let port = tokio_serial::new(com, 115200).open_native_async().unwrap();
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
            /*
            TALKER ID 	XX 	All talker IDs usable => GN here as multi GNSS solution
            SENTECE ID 	GGA
            UTC of position 	hhmmss.ss 	Fixed length 2 digits after dot
            Latitude 	llll.lllllll 	Fixed length 4 digits before and 7 after dot
            Hemisphere of latitude 	N/S 	N if value of latitude is positive
            Longitude 	lllll.lllllll 	Fixed length 5 digits before and 7 after dot
            Hemisphere of longitude 	E/W 	E if value of longitude is positive
            GPS quality indicator 	X 	0: GNSS fix not available
                1: GNSS fix valid
                4: RTK fixed ambiguities
                5: RTK float ambiguities
            Number of satellites used for positioning 	XX 	Fixed length  01 for single digits
            HDOP 	XX.X 	Variable/fixed length 1 digit after dot, variable before
            Altitude geoid height 	(-)X.XX 	Variable/fixed length 2 digits after dot, variable before
            Unit of altitude 	M
            Geoidal separation 	(-)X.XX 	Variable/fixed length 2 digits after dot, variable before
            Unit of geoidal separation 	M
            Age of differential data 		Empty field
            Differential reference station ID 		Empty field

            checksum 	*XX 	2 digits
            */

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

fn build_ui(app: &Application) {
    // Create a button with label and margins
    let button = Button::builder()
        .label("Start correction")
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();


    let port_lst= list_serial();
    let port_lst2: Vec<&str> = port_lst.iter().map(|s| s.as_str()).collect();
    let port_choice = gtk::DropDown::from_strings(port_lst2.as_slice());

    let ntrip_url = gtk::Entry::new();
    ntrip_url.set_text("http://caster.centipede.fr:2101/GDCRT");


    let grid = Grid::new();
    grid.set_row_homogeneous(true);
    grid.set_column_homogeneous(true);

    grid.attach(&gtk::Label::new(Option::Some("GNSS receiver")), 0, 0, 1, 1);
    grid.attach(&port_choice, 1, 0, 1, 1);
    grid.attach(&gtk::Label::new(Option::Some("NTRIP correction mountpoint")), 0, 1, 1, 1);
    grid.attach(&ntrip_url, 1, 1, 1, 1);
    grid.attach(&button, 0,2,2,1);


    // Connect to "clicked" signal of `button`
    button.connect_clicked( move |button| {
        let url=ntrip_url.text().to_string();
        let port=port_lst[port_choice.selected() as usize].to_string();
        tokio::spawn(start_process(url, port));

        //button.set_label("Hello World!");
        button.set_sensitive(false);
    });



    // Create a window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GPS correct")
        .child(&grid)
        .build();

    // Present window
    window.present();
}

fn list_serial() ->Vec<String>{
    let ports=tokio_serial::available_ports();
    if let Err(e)=ports {
        println!("Error {}", e);
        return Vec::new();
    }

    let port=match ports{
        Ok(p)=>p,
        Err(e)=>{
            println!("Error {}", e);
            return Vec::new();
        }
    };
    let mut ret=Vec::new();

    for p in port {
        println!("Found port {}", p.port_name);
        // Add the last part of "/" separated string into ret vector
        ret.push(p.port_name.split("/").last().unwrap().to_string());
    }

    return ret;
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

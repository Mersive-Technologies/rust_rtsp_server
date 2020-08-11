extern crate futures;
extern crate gstreamer;
use gstreamer::prelude::*;
extern crate gstreamer_app as gst_app;
extern crate gstreamer_video as gst_video;

use futures::executor::LocalPool;
use futures::prelude::*;

use anyhow::Error;
use derive_more::{Display, Error};
use std::thread;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, Display, Error)]
#[display(fmt = "Missing element {}", _0)]
struct MissingElement(#[error(not(source))] &'static str);

#[derive(Debug, Display, Error)]
#[display(fmt = "Received error from {}: {} (debug: {:?})", src, error, debug)]
struct ErrorMessage {
    src: String,
    error: String,
    debug: Option<String>,
}

const WIDTH: usize = 320;
const HEIGHT: usize = 240;

fn create_pipeline(
    remote_host: String,
    client_rtp_port: String,
    client_rtcp_port: String,
    server_rtcp_port: String,
) -> Result<gstreamer::Pipeline, Error> {

    let _pipeline_string = format!("rtpbin name=rtpman autoremove=true
               appsrc name=src ! videoconvert ! x264enc ! rtph264pay ! rtpman.send_rtp_sink_0
               rtpman.send_rtp_src_0 ! udpsink name=rtpudpsink host={} port={}
               rtpman.send_rtcp_src_0 ! udpsink name=rtcpudpsink host={} port={} sync=false async=false
               udpsrc name=rtcpudpsrc port={} ! rtpman.recv_rtcp_sink_0",
                                   remote_host,
                                   client_rtp_port,
                                   remote_host,
                                   client_rtcp_port,
                                   server_rtcp_port);

    let pipeline = gstreamer::parse_launch(&_pipeline_string).unwrap();
    let pipeline = pipeline.dynamic_cast::<gstreamer::Pipeline>().unwrap();

    let src = pipeline.get_by_name("src").unwrap();
    let appsrc = src
        .dynamic_cast::<gst_app::AppSrc>()
        .expect("Source element is expected to be an appsrc!");

    // Specify the format we want to provide as application into the pipeline
    // by creating a video info with the given format and creating caps from it for the appsrc element.
    let video_info =
        gst_video::VideoInfo::builder(gst_video::VideoFormat::Bgrx, WIDTH as u32, HEIGHT as u32)
            .fps(15)
            .build()
            .expect("Failed to create video info");

    appsrc.set_caps(Some(&video_info.to_caps().unwrap()));
    appsrc.set_property_format(gstreamer::Format::Time);

    let mut i = 0;
    thread::spawn(move || {
        loop {
            println!("Producing frame {}", i);

            let r = if i % 2 == 0 { 0 } else { 255 };
            let g = if i % 3 == 0 { 0 } else { 255 };
            let b = if i % 5 == 0 { 0 } else { 255 };

            // Create the buffer that can hold exactly one BGRx frame.
            let mut buffer = gstreamer::Buffer::with_size(video_info.size()).unwrap();
            {
                let buffer = buffer.get_mut().unwrap();
                // For each frame we produce, we set the timestamp when it should be displayed
                // (pts = presentation time stamp)
                // The autovideosink will use this information to display the frame at the right time.
                buffer.set_pts(i * (15 / 1000) * gstreamer::MSECOND);

                // At this point, buffer is only a reference to an existing memory region somewhere.
                // When we want to access its content, we have to map it while requesting the required
                // mode of access (read, read/write).
                // See: https://gstreamer.freedesktop.org/documentation/plugin-development/advanced/allocation.html
                let mut data = buffer.map_writable().unwrap();

                for p in data.as_mut_slice().chunks_mut(4) {
                    assert_eq!(p.len(), 4);
                    p[0] = b;
                    p[1] = g;
                    p[2] = r;
                    p[3] = 0;
                }
            }

            i += 1;

            // appsrc already handles the error here
            let _ = appsrc.push_buffer(buffer);
            sleep(Duration::from_millis(15 / 1000));
        }
    });

//    appsrc.set_callbacks(
//        gst_app::AppSrcCallbacks::builder()
//            .need_data(move |appsrc, _| {
//            })
//            .build(),
//    );

    Ok(pipeline)
}

async fn message_loop(bus: gstreamer::Bus) {
    let mut messages = bus.stream();
    while let Some(msg) = messages.next().await {
        use gstreamer::MessageView;
        match msg.view() {
            MessageView::Eos(..) => {
                println!("Eos!");
                break;
            },
            MessageView::Error(err) => {
                println!(
                    "Error from {:?}: {} ({:?})",
                    err.get_src().map(|s| s.get_path_string()),
                    err.get_error(),
                    err.get_debug()
                );
                break;
            },
            MessageView::StreamStatus(_) => (),
            _ => ()
        };
    }
}

pub fn serve_rtp(
    remote_host: String,
    client_rtp_port: String,
    client_rtcp_port: String,
    server_rtcp_port: String,
) {
    gstreamer::init().unwrap();

    let pipeline = create_pipeline(remote_host, client_rtp_port, client_rtcp_port, server_rtcp_port).unwrap();
    let bus = pipeline.get_bus().unwrap();

    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");

    let mut pool = LocalPool::new();
    pool.run_until(message_loop(bus));

    pipeline
        .set_state(gstreamer::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}

use std::{sync::Arc, time::Duration};

use opencv::{core::Vector, highgui::{imshow, wait_key, wait_key_def}, imgcodecs::{imdecode, IMREAD_COLOR}};
use reqwest::Client;
use tokio::sync::Mutex;
use webrtc::{api::{interceptor_registry::register_default_interceptors, media_engine::MediaEngine, APIBuilder}, data_channel::{data_channel_message::DataChannelMessage, RTCDataChannel}, ice_transport::{ice_candidate::{RTCIceCandidate, RTCIceCandidateInit}, ice_server::RTCIceServer}, interceptor::registry::Registry, peer_connection::{configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState, sdp::session_description::RTCSessionDescription}};



#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    
    let pending_candidates: Arc<Mutex<Vec<RTCIceCandidate>>> = Arc::new(Mutex::new(vec![]));
    let clnt = Client::new();

    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);

    // WebRTC connection setting up
    let config = RTCConfiguration{
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media)?;


    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .build();

    let peer_connection = Arc::new(api.new_peer_connection(config).await?);
    let pc = Arc::downgrade(&peer_connection);
    let pc3 = peer_connection.clone();
    tokio::spawn(async move{
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Some(ice) = check_candidate().await.unwrap(){
                pc3.add_ice_candidate(ice.clone()).await.unwrap();
            }
        }
    });
    peer_connection.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
        //println!("on_ice_candidate {:?}", c);
        let pc2 = pc.clone();
        let pending_candidates3 = Arc::clone(&pending_candidates);
        Box::pin(async move {
            if let Some(c) = c {
                if let Some(pc) = pc2.upgrade(){
                    let desc = pc.remote_description().await;
                    if desc.is_none() {
                        let mut cs = pending_candidates3.lock().await;
                        cs.push(c);
                    } else {
                        match signal_candidate(&c).await{
                            Ok(ices) => {

                            },
                            Err(e) => {
                                panic!("on_ice_candidate: {}", e);
                            }
                        }
                    }
                }
            }

        })
    }));
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        println!("Peer Connection State has changed: {s}");

        if s == RTCPeerConnectionState::Failed {
            // Wait until PeerConnection has had no network activity for 30 seconds or another failure. It may be reconnected using an ICE Restart.
            // Use webrtc.PeerConnectionStateDisconnected if you are interested in detecting faster timeout.
            // Note that the PeerConnection may come back from PeerConnectionStateDisconnected.
            println!("Peer Connection has gone to failed exiting");
            let _ = done_tx.try_send(());
        }

        Box::pin(async {})
    }));


    // Register data channel creation handling
    peer_connection.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        let d_label = d.label().to_owned();
        let d_id = d.id();
        println!("New DataChannel {d_label} {d_id}");

        Box::pin(async move{
            // Register channel opening handling
            let d2 =  Arc::clone(&d);
            let d_label2 = d_label.clone();
            let d_id2 = d_id;
            // d.on_open(Box::new(move || {
            //     println!("Data channel '{d_label2}'-'{d_id2}' open. Random messages will now be sent to any connected DataChannels every 5 seconds");
            //     Box::pin(async move {
            //         let mut result = Result::<usize>::Ok(0);
            //         while result.is_ok() {
            //             let timeout = tokio::time::sleep(Duration::from_secs(5));
            //             tokio::pin!(timeout);

            //             tokio::select! {
            //                 _ = timeout.as_mut() =>{
            //                     let message = math_rand_alpha(15);
            //                     println!("Sending '{message}'");
            //                     result = d2.send_text(message).await.map_err(Into::into);
            //                 }
            //             };
            //         }
            //     })
            // }));

            // Register text message handling
            d.on_message(Box::new(move |msg: DataChannelMessage| {
                let bytes = msg.data.to_vec();
                let buf: Vector<u8> = bytes.into();
                //let frm_buf = Vector::from(bytes);
                let r = imdecode(&buf, IMREAD_COLOR).unwrap();
                imshow("ClientWindow", &r).unwrap();
                wait_key(1).unwrap();
                // println!("Message from DataChannel '{d_label}': '{msg_str}'");
                Box::pin(async{})
           }));
        })
    }));

    let offer = loop {
        let r = clnt.get("http://0.0.0.0:3030/offer/0").send().await?;
        if r.status().is_success(){
            let sdp: RTCSessionDescription = r.json().await?;
            break sdp;
        } else {
            println!("Get offer reponse: {:#?}", r);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };

    peer_connection.set_remote_description(offer).await?;

    let answer = peer_connection.create_answer(None).await?;

    let r = clnt.post("http://0.0.0.0:3030/answer/0")
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&answer)?).send().await?;
    println!("Post answer reponse: {:#?}", r);

    peer_connection.set_local_description(answer).await?;

    Ok(())
}

async fn check_candidate() -> Result<Option<RTCIceCandidateInit>, Box<dyn std::error::Error>>{
    let clnt = Client::new();
    let r = clnt.get("http://0.0.0.0:3030/candidate/0")
        .send()
        .await?
    ;
    let ices: Option<RTCIceCandidateInit> = r.json().await?;
    Ok(ices)
}

async fn signal_candidate(c: &RTCIceCandidate) -> Result<Vec<RTCIceCandidateInit>, Box<dyn std::error::Error>> {
    /*println!(
        "signal_candidate Post candidate to {}",
        format!("http://{}/candidate", addr)
    );*/
    let clnt = Client::new();
    let payload = c.to_json()?;
    let r = clnt.post("http://0.0.0.0:3030/candidate/0")
        .header("content-type", "application/json; charset=utf-8")
        .body(serde_json::to_string(&payload)?)
        .send()
        .await?
    ;

    //println!("signal_candidate Response: {}", resp.status());

    Ok(r.json().await?)
}
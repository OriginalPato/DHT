//! A minimal DHT-based DNS replacement in Rust using libp2p.

use tokio::{io::{self, AsyncBufReadExt}, sync::mpsc};
use futures::StreamExt;
use libp2p::{
    identity, kad::{record::store::{MemoryStore, RecordStore}, Kademlia, KademliaConfig, Quorum, Record, RecordKey},
    swarm::{Swarm, SwarmEvent},
    tcp, noise, yamux, Transport,
    PeerId,
};
use std::error::Error;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Create a random PeerId
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {:?}", local_peer_id);

    // Create a Kademlia behavior
    let store = MemoryStore::new(local_peer_id);
    let mut kad_config = KademliaConfig::default();
    kad_config.set_query_timeout(Duration::from_secs(10));
    let kademlia = Kademlia::with_config(local_peer_id, store, kad_config);

    // Create a Swarm using SwarmBuilder
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
        .expect("Failed to create TCP transport")
        .with_behaviour(|_| kademlia)
        .expect("Failed to create behavior")
        .build();

    // Listen on all interfaces and a random port
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Process initial command line arguments
    let args: Vec<String> = std::env::args().collect();
    
    // Print usage if no arguments provided
    if args.len() <= 1 {
        println!("Usage:");
        println!("  put <key> <value>");
        println!("  get <key>");
        println!("  exit");
        println!("DHT node is running and will continue processing requests...");
    } else {
        // Process the initial command if provided
        process_command(&args, &mut swarm).await?;
    }
    
    // Main event loop - keep the node running and process commands
    println!("DHT node is running. Enter commands or wait for network events...");
    println!("Type 'exit' to quit");
    
    // Create channels for user input
    let (tx, mut rx) = mpsc::channel::<String>(100);
    
    // Spawn a task to read from stdin
    tokio::spawn(async move {
        let mut reader = io::BufReader::new(io::stdin());
        let mut line = String::new();
        
        loop {
            line.clear();
            if reader.read_line(&mut line).await.is_ok() {
                if let Err(_) = tx.send(line.trim().to_string()).await {
                    break;
                }
            } else {
                break;
            }
        }
    });
    
    // Main event loop
    loop {
        tokio::select! {
            Some(line) = rx.recv() => {
                let args: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
                
                if !args.is_empty() {
                    if args[0] == "exit" {
                        println!("Exiting...");
                        break Ok(());
                    }
                    
                    // Process the command
                    let cmd_args = std::iter::once("program".to_string())
                        .chain(args.into_iter())
                        .collect::<Vec<_>>();
                    
                    if let Err(e) = process_command(&cmd_args, &mut swarm).await {
                        println!("Error processing command: {}", e);
                    }
                }
            },
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {:?}", address);
                    }
                    SwarmEvent::Behaviour(event) => {
                        println!("Network event received: {:?}", event);
                    }
                    _ => {}
                }
            }
        }
    }
}

// Process a command based on the provided arguments
async fn process_command(args: &[String], swarm: &mut Swarm<Kademlia<MemoryStore>>) -> Result<(), Box<dyn Error>> {
    if args.len() > 3 && args[1] == "put" {
        let key_string = args[2].clone();
        let value = args[3].clone().into_bytes();
        
        // Convert string to bytes for the key
        let key_bytes = key_string.as_bytes().to_vec();
        let record_key = RecordKey::new(&key_bytes);
        
        // Create a record for local storage
        let record = Record::new(record_key.clone(), value.clone());
        
        // Store the record locally without requiring a quorum
        swarm.behaviour_mut().store_mut().put(record)?;
        println!("Record stored locally for key: {}", key_string);
        
        // Also publish to the DHT
        let record_for_dht = Record::new(record_key, value);
        match swarm.behaviour_mut().put_record(record_for_dht, Quorum::One) {
            Ok(_) => println!("Publishing record to DHT for key: {}", key_string),
            Err(e) => println!("Error publishing to DHT: {}", e),
        };
    } else if args.len() > 2 && args[1] == "get" {
        let key_string = args[2].clone();
        
        // Convert string to bytes for the key
        let key_bytes = key_string.as_bytes().to_vec();
        let record_key = RecordKey::new(&key_bytes);
        
        // Try to get the record from the local store
        if let Some(record) = swarm.behaviour_mut().store_mut().get(&record_key) {
            println!("Found record locally: {} => {}", 
                key_string,
                String::from_utf8_lossy(&record.value));
        } else {
            println!("Record not found locally for key: {}", key_string);
            // Fall back to DHT query
            let query_id = swarm.behaviour_mut().get_record(record_key);
            println!("Querying DHT for key: {} (Query ID: {:?})", key_string, query_id);
        }
    } else {
        println!("Usage:");
        println!("  put <key> <value>");
        println!("  get <key>");
    }
    
    Ok(())
}

use crossbeam_channel::Sender;
use raft_core::commands::{RpcCommand, ServerCommand};
use raft_proto::rpc::key_value_service_server::KeyValueService;
use raft_proto::rpc::{GetRequest, GetResponse, PutRequest, PutResponse};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};

pub struct RaftKvService {
    // Channel to send commands to RaftDriver
    pub cmd_tx: Sender<ServerCommand>,
    // Map of peer IDs to addresses for redirection
    pub peers_map: HashMap<u64, SocketAddr>,
}

#[tonic::async_trait]
impl KeyValueService for RaftKvService {
    async fn put(&self, request: Request<PutRequest>) -> Result<Response<PutResponse>, Status> {
        let req = request.into_inner();
        let (tx, rx) = oneshot::channel();

        let cmd = RpcCommand::Put {
            key: req.key,
            value: req.value,
            resp: tx,
        };

        // Send to RaftDriver
        if self.cmd_tx.send(ServerCommand::Rpc(cmd)).is_err() {
            return Err(Status::internal("Raft driver channel closed"));
        }

        // Wait for response
        match rx.await {
            Ok(Ok(_)) => Ok(Response::new(PutResponse {})),
            Ok(Err(e)) if e.starts_with("Not leader") => {
                let mut status = Status::failed_precondition("Not the leader");

                // Extract leader ID
                if let Some(id_str) = e.strip_prefix("Not leader: ") {
                    if let Ok(leader_id) = id_str.trim().parse::<u64>() {
                        if let Some(addr) = self.peers_map.get(&leader_id) {
                            status
                                .metadata_mut()
                                .insert("x-raft-leader-id", leader_id.to_string().parse().unwrap());
                            status
                                .metadata_mut()
                                .insert("x-raft-leader-addr", addr.to_string().parse().unwrap());
                        }
                    }
                }

                Err(status)
            }
            Ok(Err(e)) => Err(Status::internal(format!("Raft error: {}", e))),
            Err(_) => Err(Status::internal("Raft driver dropped callback")),
        }
    }

    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let req = request.into_inner();
        let (tx, rx) = oneshot::channel();

        let cmd = RpcCommand::Get {
            key: req.key,
            resp: tx,
        };

        if self.cmd_tx.send(ServerCommand::Rpc(cmd)).is_err() {
            return Err(Status::internal("Raft driver channel closed"));
        }

        match rx.await {
            Ok(Some(val)) => Ok(Response::new(GetResponse {
                value: val,
                found: true,
            })),
            Ok(None) => Ok(Response::new(GetResponse {
                value: "".into(),
                found: false,
            })),
            Err(_) => Err(Status::internal("Raft driver dropped callback")),
        }
    }
}

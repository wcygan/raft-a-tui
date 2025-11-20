fn main() {
    // Compile KV proto (Prost)
    prost_build::compile_protos(&["proto/kv.proto"], &["proto"]).unwrap();
    
    // Compile RPC proto (Tonic)
    tonic_build::compile_protos("proto/rpc.proto").unwrap();
}

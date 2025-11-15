use prost::Message;

/// Encode any prost Message into a Vec<u8>.
pub fn encode<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut buf).expect("prost encode");
    buf
}

/// Decode any prost Message from &[u8].
pub fn decode<M: Message + Default>(bytes: &[u8]) -> Result<M, prost::DecodeError> {
    M::decode(bytes)
}

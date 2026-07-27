#![no_main]

use libfuzzer_sys::fuzz_target;
use quick_share_protocol::{ChunkData, InfoRequest, MessageType, RequestId};
use quick_share_transfer::noise::{
    ApplicationFrame, decode_control_frame, fuzz_decode_segment, read_record,
};
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let _ = read_record(&mut Cursor::new(data));
    let _ = fuzz_decode_segment(data);
    let frame = ApplicationFrame {
        message_type: MessageType::InfoRequest,
        request_id: RequestId::from_bytes([0; 16]),
        payload: data.to_vec(),
    };
    let _ = decode_control_frame::<InfoRequest>(&frame, MessageType::InfoRequest);
    let chunk_frame = ApplicationFrame {
        message_type: MessageType::ChunkData,
        request_id: RequestId::from_bytes([1; 16]),
        payload: data.to_vec(),
    };
    let _ = decode_control_frame::<ChunkData>(&chunk_frame, MessageType::ChunkData);
});

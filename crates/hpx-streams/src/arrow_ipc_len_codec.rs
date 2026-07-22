use arrow::{array::RecordBatch, ipc::reader::StreamDecoder};
use bytes::{Buf, BytesMut};

use crate::{StreamBodyError, error::StreamBodyKind};

#[derive(Debug)]
pub(crate) struct ArrowIpcCodec {
    max_length: usize,
    decoder: StreamDecoder,
    current_obj_len: usize,
}

impl ArrowIpcCodec {
    pub(crate) fn new_with_max_length(max_length: usize) -> Self {
        Self {
            max_length,
            decoder: StreamDecoder::new(),
            current_obj_len: 0,
        }
    }
}

impl tokio_util::codec::Decoder for ArrowIpcCodec {
    type Item = RecordBatch;
    type Error = StreamBodyError;

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<RecordBatch>, StreamBodyError> {
        let buf_len = buf.len();
        if buf_len == 0 {
            return Ok(None);
        }

        let obj_bytes = buf.as_ref();
        let obj_bytes_len = obj_bytes.len();
        let mut buffer = arrow::buffer::Buffer::from(obj_bytes);
        let maybe_record = self.decoder.decode(&mut buffer).map_err(|e| {
            StreamBodyError::new(
                StreamBodyKind::CodecError,
                Some(Box::new(e)),
                Some("Decode arrow IPC record error".into()),
            )
        })?;

        if maybe_record.is_none() {
            self.current_obj_len += obj_bytes_len;
        } else {
            self.current_obj_len = 0;
        }

        if self.current_obj_len > self.max_length {
            return Err(StreamBodyError::new(
                StreamBodyKind::CodecError,
                None,
                Some("Object length exceeds the maximum length".into()),
            ));
        }

        buf.advance(obj_bytes_len - buffer.len());
        Ok(maybe_record)
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<RecordBatch>, StreamBodyError> {
        self.decode(buf)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Int32Array, StringArray},
        datatypes::{DataType, Field, Schema},
        ipc::writer::StreamWriter,
    };
    use tokio_util::codec::Decoder;

    use super::*;

    fn make_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Int32, false),
        ]));
        let names = Arc::new(StringArray::from(vec!["alice", "bob"]));
        let values = Arc::new(Int32Array::from(vec![1, 2]));
        RecordBatch::try_new(schema, vec![names, values]).unwrap()
    }

    fn encode_ipc(batch: &RecordBatch) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut buf, batch.schema().as_ref()).unwrap();
            writer.write(batch).unwrap();
            writer.finish().unwrap();
        }
        buf
    }

    #[test]
    fn normal_parse_single_batch() {
        let batch = make_batch();
        let data = encode_ipc(&batch);

        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..]);
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.num_rows(), 2);
        assert_eq!(result.num_columns(), 2);
    }

    #[test]
    fn empty_input_returns_none() {
        let mut codec = ArrowIpcCodec::new_with_max_length(1024);
        let mut buf = BytesMut::new();
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn truncated_payload_returns_none() {
        let batch = make_batch();
        let data = encode_ipc(&batch);

        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..4]);
        let result = codec.decode(&mut buf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn garbage_data_returns_none() {
        let mut codec = ArrowIpcCodec::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"not arrow"[..]);
        // Arrow StreamDecoder returns None for unrecognized data, not an error
        let result = codec.decode(&mut buf).unwrap();
        assert!(result.is_none());
    }
}

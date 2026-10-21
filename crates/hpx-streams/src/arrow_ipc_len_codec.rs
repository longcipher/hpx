use arrow::{array::RecordBatch, ipc::reader::StreamDecoder};
use bytes::{Buf, BytesMut};

use crate::{StreamBodyError, error::StreamBodyKind};

#[derive(Debug)]
pub(crate) struct ArrowIpcCodec {
    max_length: usize,
    decoder: StreamDecoder,
    /// Number of bytes received for the object that is currently being
    /// decoded (reset whenever a record was decoded).
    current_obj_len: usize,
    /// Number of bytes left in the read buffer after the previous call.
    ///
    /// Used to tell how many bytes were *newly* received since the previous
    /// call: the read buffer keeps the undecoded bytes, so its length alone
    /// would count them again on every call.
    pending_after_last: usize,
}

impl ArrowIpcCodec {
    pub(crate) fn new_with_max_length(max_length: usize) -> Self {
        Self {
            max_length,
            decoder: StreamDecoder::new(),
            current_obj_len: 0,
            pending_after_last: 0,
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
            self.current_obj_len = 0;
            self.pending_after_last = 0;
            return Ok(None);
        }

        // Only the bytes that arrived since the previous call belong to the
        // object that is currently being decoded. Counting the whole buffer
        // again would count the already-buffered bytes once per chunk, so a
        // message fed in N chunks was counted N times and tripped the
        // max-length guard although it fits.
        self.current_obj_len += buf_len.saturating_sub(self.pending_after_last);

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

        // Advance the read buffer by the bytes the Arrow decoder consumed.
        let consumed = obj_bytes_len.saturating_sub(buffer.len());
        buf.advance(consumed);
        self.pending_after_last = buf.len();

        if maybe_record.is_some() {
            // The record was decoded: the next object starts from scratch.
            self.current_obj_len = 0;
        }

        if self.current_obj_len > self.max_length {
            return Err(StreamBodyError::new(
                StreamBodyKind::CodecError,
                None,
                Some("Object length exceeds the maximum length".into()),
            ));
        }

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

    #[test]
    fn decode_eof_returns_none_on_empty() {
        let mut codec = ArrowIpcCodec::new_with_max_length(1024);
        let mut buf = BytesMut::new();
        assert!(matches!(codec.decode_eof(&mut buf), Ok(None)));
    }

    #[test]
    fn decode_eof_returns_pending_batch() {
        let batch = make_batch();
        let data = encode_ipc(&batch);

        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..]);
        let result = codec.decode_eof(&mut buf).unwrap();
        assert!(result.is_some(), "decode_eof must flush a pending batch");
    }

    #[test]
    fn garbage_exceeding_max_length_errors() {
        // Arrow StreamDecoder returns None for garbage; the accumulated
        // consumed length then trips the max-length guard.
        let data = b"not arrow data";
        let mut codec = ArrowIpcCodec::new_with_max_length(4);
        let mut buf = BytesMut::from(&data[..]);
        assert!(
            codec.decode(&mut buf).is_err(),
            "garbage over max_length must error"
        );
    }

    #[test]
    fn garbage_equal_to_max_length_is_accepted() {
        // Exactly at the limit must still be accepted (guard is `>` not `>=`).
        let data = b"not arrow data!";
        let mut codec = ArrowIpcCodec::new_with_max_length(data.len());
        let mut buf = BytesMut::from(&data[..]);
        assert!(
            !codec.decode(&mut buf).is_err(),
            "exactly-at-limit input must not error"
        );
    }

    #[test]
    fn single_column_batch() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let values = Arc::new(Int32Array::from(vec![10, 20, 30]));
        let batch = RecordBatch::try_new(schema, vec![values]).unwrap();

        let data = encode_ipc(&batch);
        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..]);
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.num_rows(), 3);
        assert_eq!(result.num_columns(), 1);
    }

    #[test]
    fn empty_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Int32, false),
        ]));
        let names = Arc::new(StringArray::from(Vec::<&str>::new()));
        let values = Arc::new(Int32Array::from(Vec::<i32>::new()));
        let batch = RecordBatch::try_new(schema, vec![names, values]).unwrap();

        let data = encode_ipc(&batch);
        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..]);
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.num_rows(), 0);
        assert_eq!(result.num_columns(), 2);
    }

    #[test]
    fn large_batch() {
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let values: Vec<i64> = (0..1000).collect();
        let array = Arc::new(arrow::array::Int64Array::from(values));
        let batch = RecordBatch::try_new(schema, vec![array]).unwrap();

        let data = encode_ipc(&batch);
        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..]);
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.num_rows(), 1000);
    }

    #[test]
    fn incremental_feed_partial_data() {
        let batch = make_batch();
        let data = encode_ipc(&batch);

        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);

        // Feed half the data
        let half = data.len() / 2;
        let mut buf = BytesMut::from(&data[..half]);
        let result = codec.decode(&mut buf).unwrap();
        assert!(result.is_none());

        // Feed the rest
        buf.extend_from_slice(&data[half..]);
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.num_rows(), 2);
    }

    #[test]
    fn chunked_feeds_do_not_double_count_the_pending_length() {
        // Regression: the pending length was accumulated on every decode call,
        // so a message fed in many chunks was counted once per chunk and
        // tripped the max-length guard although it fits comfortably.
        let batch = make_batch();
        let data = encode_ipc(&batch);

        let mut codec = ArrowIpcCodec::new_with_max_length(data.len() + 16);
        let mut buf = BytesMut::new();
        let mut decoded = None;
        for chunk in data.chunks(8) {
            buf.extend_from_slice(chunk);
            decoded = codec.decode(&mut buf).unwrap();
            if decoded.is_some() {
                break;
            }
        }

        let decoded = decoded.expect("the batch must be decoded");
        assert_eq!(decoded.num_rows(), 2);
    }

    #[test]
    fn multi_column_batch_preserves_data() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Int32, false),
        ]));
        let names = Arc::new(StringArray::from(vec!["alice", "bob", "charlie"]));
        let values = Arc::new(Int32Array::from(vec![100, 200, 300]));
        let batch = RecordBatch::try_new(schema, vec![names, values]).unwrap();

        let data = encode_ipc(&batch);
        let mut codec = ArrowIpcCodec::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(&data[..]);
        let result = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(result.num_rows(), 3);
        assert_eq!(result.num_columns(), 2);

        let name_col = result
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(name_col.value(0), "alice");
        assert_eq!(name_col.value(2), "charlie");
    }
}

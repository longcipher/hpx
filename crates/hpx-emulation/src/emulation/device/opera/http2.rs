macro_rules! headers_stream_dependency {
    () => {
        StreamDependency::new(StreamId::zero(), 219, true)
    };
}

macro_rules! http2_options {
    () => {
        Http2Options::builder()
            .initial_window_size(6_291_456)
            .initial_connection_window_size(15_728_640)
            .max_header_list_size(262_144)
            .header_table_size(65536)
            .enable_push(false)
            .headers_stream_dependency(headers_stream_dependency!())
            .headers_pseudo_order(pseudo_order!())
            .settings_order(settings_order!())
            .build()
    };
}

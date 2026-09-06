macro_rules! headers_stream_dependency {
    () => {
        StreamDependency::new(StreamId::zero(), 219, true)
    };
}

macro_rules! http2_options {
    (@base $builder:expr) => {
        $builder
            .initial_window_size(6_291_456)
            .initial_connection_window_size(15_728_640)
            .max_header_list_size(262_144)
            .header_table_size(65536)
            .headers_stream_dependency(headers_stream_dependency!())
            .headers_pseudo_order(pseudo_order!())
            .settings_order(settings_order!())
    };

    (1) => {
        http2_options!(@base Http2Options::builder())
            .max_concurrent_streams(1000)
            .build()
    };
    (2) => {
        http2_options!(@base Http2Options::builder())
            .max_concurrent_streams(1000)
            .enable_push(false)
            .build()
    };
    (3) => {
        http2_options!(@base Http2Options::builder())
            .enable_push(false)
            .build()
    };
}

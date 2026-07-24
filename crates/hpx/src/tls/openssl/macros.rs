macro_rules! set_bool {
    ($cfg:expr, !$field:ident, $conn:expr, $setter:ident, $arg:expr) => {
        if !$cfg.$field {
            let _ = $conn.$setter($arg);
        }
    };
}

macro_rules! set_option_ref_try {
    ($cfg:expr, $field:ident, $conn:expr, $setter:ident) => {
        if let Some(val) = $cfg.$field.as_ref() {
            $conn.$setter(val).map_err(Error::tls)?;
        }
    };
}

macro_rules! set_option_inner_try {
    ($field:ident, $conn:expr, $setter:ident) => {
        $conn
            .$setter($field.map(|v| v.to_native_version()))
            .map_err(Error::tls)?;
    };
}

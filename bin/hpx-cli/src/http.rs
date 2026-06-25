use std::{
    io::{self, Read as _},
    time::Duration,
};

use eyre::{OptionExt, Result, WrapErr};
use hpx::Client;

use crate::{
    cli::{Cli, FormValue, OutputFormat},
    output::{
        TimingWaterfall, copy_to_clipboard, format_json_pretty, print_headers,
        print_redirect_history, print_request_line, print_status_line, write_body,
    },
};

pub(crate) async fn execute(cli: &Cli) -> Result<()> {
    let url = cli.url.as_deref().ok_or_eyre("no URL provided")?;
    let method_str = cli.method.as_str();
    let method = hpx_method(cli.method);

    let mut client_builder = Client::builder();

    // Proxy
    if let Some(ref proxy_url) = cli.proxy {
        let proxy = hpx::Proxy::all(proxy_url)
            .wrap_err_with(|| format!("invalid proxy URL: {proxy_url}"))?;
        client_builder = client_builder.proxy(proxy);
    }

    // Cookie store
    if !cli.cookie.is_empty() || cli.cookie_jar.is_some() {
        client_builder = client_builder.cookie_store(true);
    }

    let client = client_builder.build()?;
    let mut builder = client.request(method, url.to_string());

    for (name, value) in cli.parsed_headers() {
        builder = builder.header(name, value);
    }

    // Manual cookies (add before request)
    for (name, value) in cli.parsed_cookies() {
        let cookie_str = format!("{name}={value}");
        builder = builder.header("Cookie", &cookie_str);
    }

    // Request body: --data or --json
    if let Some(ref data) = cli.data {
        let body = if data == "@-" {
            let mut buf = Vec::new();
            io::stdin()
                .read_to_end(&mut buf)
                .wrap_err("failed to read body from stdin")?;
            buf
        } else if let Some(stripped) = data.strip_prefix('@') {
            std::fs::read(stripped)
                .wrap_err_with(|| format!("failed to read body from file {stripped}"))?
        } else {
            data.as_bytes().to_vec()
        };
        builder = builder.body(body);
    }

    if let Some(ref json) = cli.json {
        let body = if json == "@-" {
            let mut buf = Vec::new();
            io::stdin()
                .read_to_end(&mut buf)
                .wrap_err("failed to read JSON body from stdin")?;
            buf
        } else if let Some(stripped) = json.strip_prefix('@') {
            std::fs::read(stripped)
                .wrap_err_with(|| format!("failed to read JSON body from file {stripped}"))?
        } else {
            json.as_bytes().to_vec()
        };
        builder = builder.header("Content-Type", "application/json");
        builder = builder.body(body);
    }

    // Form data
    if !cli.form.is_empty() {
        if cli.has_form_file_references() {
            // Use multipart form when any field has file references
            let mut form = hpx::multipart::Form::new();
            for (key, value) in cli.parsed_form_fields_with_files() {
                match value {
                    FormValue::Text(text) => {
                        form = form.text(key, text);
                    }
                    FormValue::File(path) => {
                        let static_key: &'static str = Box::leak(key.into_boxed_str());
                        form = form.file(static_key, &path).await.wrap_err_with(|| {
                            format!("failed to read file {path} for form field")
                        })?;
                    }
                }
            }
            builder = builder.multipart(form);
        } else {
            // Plain urlencoded form data
            let fields: Vec<(String, String)> = cli.parsed_form_fields();
            builder = builder.form(&fields);
        }
    }

    // Multipart form data
    if !cli.multipart.is_empty() || !cli.multipart_file.is_empty() {
        let mut form = hpx::multipart::Form::new();
        for (key, value) in cli.parsed_multipart_fields() {
            form = form.text(key, value);
        }
        for (key, path) in cli.parsed_multipart_files() {
            // Leak the key to get 'static str for hpx multipart API
            let static_key: &'static str = Box::leak(key.into_boxed_str());
            form = form
                .file(static_key, &path)
                .await
                .wrap_err_with(|| format!("failed to add file {path} to multipart form"))?;
        }
        builder = builder.multipart(form);
    }

    if let Some(ref basic) = cli.basic {
        let (user, pass) = basic
            .split_once(':')
            .ok_or_eyre("invalid basic auth format, expected USER:PASS")?;
        builder = builder.basic_auth(user, Some(pass));
    }

    if let Some(ref bearer) = cli.bearer {
        builder = builder.bearer_auth(bearer);
    }

    if let Some(secs) = cli.timeout {
        builder = builder.timeout(Duration::from_secs_f64(secs));
    }

    if cli.dry_run {
        print_request_line(method_str, url, false);
        for (name, value) in cli.parsed_headers() {
            print_headers(&[(name, value)], true, false);
        }
        if !cli.form.is_empty() {
            eprintln!("[form data] {} fields", cli.form.len());
        }
        if !cli.multipart.is_empty() || !cli.multipart_file.is_empty() {
            let count = cli.multipart.len() + cli.multipart_file.len();
            eprintln!("[multipart] {count} fields");
        }
        if let Some(ref proxy) = cli.proxy {
            eprintln!("[proxy] {proxy}");
        }
        if cli.retry > 0 {
            eprintln!("[retry] {} attempts", cli.retry);
        }
        return Ok(());
    }

    let verbose = cli.verbose > 0;
    let color = crate::output::use_color(cli.color, crate::output::is_terminal());

    if verbose {
        print_request_line(method_str, url, color);
    }

    let mut waterfall = TimingWaterfall::new();

    let response = builder.send().await?;

    waterfall.mark_request_done();

    let status = response.status();
    let status_code = status.as_u16();
    let version = format!("{:?}", response.version());

    // Extract data before consuming response
    let response_headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let set_cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();
    let extensions = response.extensions().clone();

    // Display redirect history
    print_redirect_history(&extensions, verbose, color);

    if verbose || !cli.silent {
        print_status_line(status_code, &version, color);
        if verbose {
            print_headers(&response_headers, verbose, color);
        }
    }

    let bytes = response.bytes().await?;

    waterfall.mark_body_done();

    // Save cookies to jar if requested
    if let Some(ref jar_path) = cli.cookie_jar
        && !set_cookies.is_empty()
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(jar_path)?;
        for cookie_str in &set_cookies {
            writeln!(file, "{cookie_str}")?;
        }
        if verbose {
            eprintln!(
                "[cookie-jar] saved {} cookie(s) to {jar_path}",
                set_cookies.len()
            );
        }
    }

    let output_path = cli.output.as_deref();

    let body_bytes = match cli.format {
        OutputFormat::Json => {
            let pretty = format_json_pretty(&bytes)?;
            pretty.into_bytes()
        }
        OutputFormat::Auto => {
            if crate::output::looks_like_json_str(std::str::from_utf8(&bytes).unwrap_or("")) {
                match format_json_pretty(&bytes) {
                    Ok(pretty) => pretty.into_bytes(),
                    Err(_) => bytes.to_vec(),
                }
            } else {
                bytes.to_vec()
            }
        }
        OutputFormat::Text | OutputFormat::Raw => bytes.to_vec(),
    };

    if cli.clipboard {
        if let Ok(text) = std::str::from_utf8(&body_bytes) {
            copy_to_clipboard(text)?;
            if !cli.silent {
                eprintln!("Response copied to clipboard");
            }
        } else {
            eprintln!("Warning: response body is not valid UTF-8, cannot copy to clipboard");
        }
    }

    if cli.timing {
        waterfall.print(color);
    }

    write_body(&body_bytes, output_path)?;

    Ok(())
}

const fn hpx_method(method: crate::cli::Method) -> hpx::Method {
    match method {
        crate::cli::Method::Get => hpx::Method::GET,
        crate::cli::Method::Post => hpx::Method::POST,
        crate::cli::Method::Put => hpx::Method::PUT,
        crate::cli::Method::Delete => hpx::Method::DELETE,
        crate::cli::Method::Patch => hpx::Method::PATCH,
        crate::cli::Method::Head => hpx::Method::HEAD,
        crate::cli::Method::Options => hpx::Method::OPTIONS,
    }
}

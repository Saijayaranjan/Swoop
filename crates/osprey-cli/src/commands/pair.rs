//! `pair`: start device pairing and print the code plus a QR code rendered as text.

use osprey_services::api::PairingInfo;
use qrcode::render::unicode;
use qrcode::QrCode;

use crate::error::{CliError, CliResult};
use crate::output::print_json;
use crate::transport::Client;

pub async fn run(client: &Client, json: bool) -> CliResult<()> {
    let body = serde_json::json!({ "scopes": ["read", "add", "control"] });
    let value = client.post("/api/v1/devices/pairing", body).await?;
    let info: PairingInfo = serde_json::from_value(value)?;

    if json {
        print_json(&info);
        return Ok(());
    }

    println!("Pairing code: {}", info.code);
    println!("Expires:      {}", info.expires_at.to_datetime().to_rfc3339());
    if let Some(fp) = &info.tls_fingerprint {
        println!("TLS fingerprint: {fp}");
    }
    println!();
    match QrCode::new(info.url.as_bytes()) {
        Ok(code) => {
            let image = code
                .render::<unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            println!("{image}");
        }
        Err(e) => {
            return Err(CliError::Other(anyhow::anyhow!(
                "failed to render pairing QR code: {e}"
            )))
        }
    }
    println!("{}", info.url);
    Ok(())
}

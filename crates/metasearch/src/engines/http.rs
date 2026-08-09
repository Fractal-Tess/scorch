use futures_util::StreamExt;

use crate::{Error, Result};

pub async fn read_limited(
    response: reqwest::Response,
    engine: &'static str,
    limit: usize,
) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::ResponseTooLarge { engine, limit });
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| Error::Request {
            engine,
            message: error.to_string(),
        })?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(Error::ResponseTooLarge { engine, limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

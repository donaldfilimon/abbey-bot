//! Bounded HTTP response-body reads shared by the network adapters.

use futures_util::StreamExt as _;

#[derive(Debug)]
pub enum BodyReadError {
    TooLarge { max: usize },
    Read(String),
}

impl BodyReadError {
    pub fn is_too_large(&self) -> bool {
        matches!(self, Self::TooLarge { .. })
    }
}

impl std::fmt::Display for BodyReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { max } => write!(f, "response body exceeds the {max}-byte limit"),
            Self::Read(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BodyReadError {}

fn append_capped(body: &mut Vec<u8>, chunk: &[u8], max: usize) -> Result<(), BodyReadError> {
    if chunk.len() > max.saturating_sub(body.len()) {
        return Err(BodyReadError::TooLarge { max });
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// Read a response incrementally and stop before retaining more than `max`
/// bytes. `Content-Length` is only an early rejection; the chunk accounting is
/// authoritative because remote servers can omit or misstate it.
pub async fn read_capped(
    response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, BodyReadError> {
    if response
        .content_length()
        .is_some_and(|len| usize::try_from(len).map_or(true, |len| len > max))
    {
        return Err(BodyReadError::TooLarge { max });
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            BodyReadError::Read(format!(
                "reading the response failed: {}",
                error.without_url()
            ))
        })?;
        append_capped(&mut body, &chunk, max)?;
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_chunks_cannot_cross_the_limit() {
        let mut body = Vec::new();
        append_capped(&mut body, b"1234", 5).unwrap();
        let error = append_capped(&mut body, b"56", 5).unwrap_err();
        assert_eq!(body, b"1234", "the rejected chunk is never retained");
        assert!(error.to_string().contains("5-byte limit"));
    }
}

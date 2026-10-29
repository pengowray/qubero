//! The envelope every reading is answered in: done, waiting on chunks, still working, or failed.

use crate::*;

#[derive(Serialize)]
#[serde(tag = "status")]
pub(crate) enum Reply<T: Serialize> {
    #[serde(rename = "ok")]
    Ok {
        node: T,
        /// Chunks the answer was given without: previews that had not arrived.
        /// Fetching these and asking again fills them in.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        wanted: Vec<f64>,
    },
    #[serde(rename = "pending")]
    Pending { chunks: Vec<f64>, reached_bytes: f64 },
    #[serde(rename = "working")]
    Working { reached_bytes: f64 },
    #[serde(rename = "error")]
    Error { message: String },
}

pub(crate) fn wanted(e: &Evaluator) -> Vec<f64> {
    e.wanted().into_iter().map(|m| m.chunk as f64).collect()
}

pub(crate) fn reply_with<T: Serialize>(r: Result<T, EvalError>, reached_bytes: f64, wanted: Vec<f64>) -> String {
    let rep = match r {
        Ok(node) => Reply::Ok { node, wanted },
        Err(EvalError::Pending(m)) => {
            Reply::Pending { chunks: m.into_iter().map(|m| m.chunk as f64).collect(), reached_bytes }
        }
        Err(EvalError::Busy { reached_bits }) => Reply::Working { reached_bytes: (reached_bits / 8) as f64 },
        Err(EvalError::Failed(message)) => Reply::Error { message },
    };
    serde_json::to_string(&rep).unwrap_or_else(|e| format!("{{\"status\":\"error\",\"message\":{:?}}}", e.to_string()))
}

/// The same, for the callers with nothing further to say.
pub(crate) fn reply<T: Serialize>(r: Result<T, EvalError>) -> String {
    reply_with(r, 0.0, Vec::new())
}

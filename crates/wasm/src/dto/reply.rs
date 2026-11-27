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
    Pending {
        chunks: Vec<f64>,
        reached_bytes: f64,
        /// For a slice of children whose rest wait on a pass that waits for
        /// bytes of the file: the ones before the first that waits, as for
        /// `Working`. See `slice_reply`.
        #[serde(skip_serializing_if = "Option::is_none")]
        ready: Option<T>,
        /// Chunks `ready` was given without, as for `Ok`.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        wanted: Vec<f64>,
        /// How far that pass has got.
        #[serde(skip_serializing_if = "Option::is_none")]
        unpacking: Option<UnpackingDto>,
    },
    #[serde(rename = "working")]
    Working {
        reached_bytes: f64,
        /// The part of the answer that did not wait: for a slice of children,
        /// the ones before the first that waits. See `slice_reply`.
        #[serde(skip_serializing_if = "Option::is_none")]
        ready: Option<T>,
        /// Chunks `ready` was given without, as for `Ok`.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        wanted: Vec<f64>,
        /// How far the pass this call ran out in has got, when it ran out in
        /// one: what the rest of the answer waits on.
        #[serde(skip_serializing_if = "Option::is_none")]
        unpacking: Option<UnpackingDto>,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

pub(crate) fn wanted(e: &Evaluator) -> Vec<f64> {
    e.wanted().into_iter().map(|m| m.chunk as f64).collect()
}

/// How far a pass has got, as the web reads it.
pub(crate) fn unpacking_dto(p: Unpacking) -> UnpackingDto {
    UnpackingDto {
        run_space: p.run_space,
        run_offset_bits: p.run_offset_bits as f64,
        read_bits: p.read_bits as f64,
        run_bits: p.run_bits as f64,
        unpacked_bytes: p.unpacked_bytes as f64,
    }
}

pub(crate) fn reply_with<T: Serialize>(r: Result<T, EvalError>, reached_bytes: f64, wanted: Vec<f64>) -> String {
    let rep = match r {
        Ok(node) => Reply::Ok { node, wanted },
        Err(EvalError::Pending(m)) => Reply::Pending {
            chunks: m.into_iter().map(|m| m.chunk as f64).collect(),
            reached_bytes,
            ready: None,
            wanted: Vec::new(),
            unpacking: None,
        },
        Err(EvalError::Busy { reached_bits }) => {
            Reply::Working { reached_bytes: (reached_bits / 8) as f64, ready: None, wanted: Vec::new(), unpacking: None }
        }
        Err(EvalError::Failed(message)) => Reply::Error { message },
    };
    to_json(&rep)
}

/// A slice of children answered as far as it could be. When the rest wait on
/// more work, the "working" reply carries the ones before the first that
/// waits as `ready`, and `unpacking` says how far the pass the slice waits on
/// has got. A slice that waits on bytes of the file is answered whole, as
/// before: the listing's walk over a file still loading was tuned to stop at
/// such a slice, and reading on into the children it has would ask for more
/// chunks at once. The exception is a pass that reads its run a window at a
/// time and waits for the next window: that wait comes between goes the way
/// running out of time does, so it is answered the same way, or the rows
/// before the stream and the progress would come and go with each window.
pub(crate) fn slice_reply<T: Serialize>(ready: Vec<T>, rest: Result<(), EvalError>, e: &Evaluator) -> String {
    let wanted = wanted(e);
    let rep = match rest {
        Ok(()) => Reply::Ok { node: ready, wanted },
        Err(EvalError::Pending(m)) => {
            let chunks = m.into_iter().map(|m| m.chunk as f64).collect();
            let reached_bytes = (e.reached_bits() / 8) as f64;
            match e.unpacking() {
                Some(p) => {
                    let (ready, wanted) = if ready.is_empty() { (None, Vec::new()) } else { (Some(ready), wanted) };
                    Reply::Pending { chunks, reached_bytes, ready, wanted, unpacking: Some(unpacking_dto(p)) }
                }
                None => Reply::Pending { chunks, reached_bytes, ready: None, wanted: Vec::new(), unpacking: None },
            }
        }
        Err(EvalError::Busy { reached_bits }) => {
            let (ready, wanted) = if ready.is_empty() { (None, Vec::new()) } else { (Some(ready), wanted) };
            Reply::Working { reached_bytes: (reached_bits / 8) as f64, ready, wanted, unpacking: e.unpacking().map(unpacking_dto) }
        }
        Err(EvalError::Failed(message)) => Reply::Error { message },
    };
    to_json(&rep)
}

fn to_json<T: Serialize>(rep: &Reply<T>) -> String {
    serde_json::to_string(rep).unwrap_or_else(|e| format!("{{\"status\":\"error\",\"message\":{:?}}}", e.to_string()))
}

/// The same, for the callers with nothing further to say.
pub(crate) fn reply<T: Serialize>(r: Result<T, EvalError>) -> String {
    reply_with(r, 0.0, Vec::new())
}

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
        /// The part of the answer that did not wait: for a slice of children,
        /// the ones before the first that waits. See `slice_reply`.
        #[serde(skip_serializing_if = "Option::is_none")]
        ready: Option<T>,
        /// Chunks `ready` was given without, as for `Ok`.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        wanted: Vec<f64>,
    },
    #[serde(rename = "working")]
    Working {
        reached_bytes: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ready: Option<T>,
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
        },
        Err(EvalError::Busy { reached_bits }) => {
            Reply::Working { reached_bytes: (reached_bits / 8) as f64, ready: None, wanted: Vec::new(), unpacking: None }
        }
        Err(EvalError::Failed(message)) => Reply::Error { message },
    };
    to_json(&rep)
}

/// A slice of children answered as far as it could be: all of them, or the
/// ones before the first that waits, carried on the "pending" or "working"
/// reply as `ready`, with `unpacking` saying how far the pass the slice
/// waits on has got. A slice with none ready is answered as before.
pub(crate) fn slice_reply<T: Serialize>(ready: Vec<T>, rest: Result<(), EvalError>, e: &Evaluator) -> String {
    let wanted = wanted(e);
    let reached_bytes = (e.reached_bits() / 8) as f64;
    let ready_part = |ready: Vec<T>, wanted: Vec<f64>| if ready.is_empty() { (None, Vec::new()) } else { (Some(ready), wanted) };
    let rep = match rest {
        Ok(()) => Reply::Ok { node: ready, wanted },
        Err(EvalError::Pending(m)) => {
            let (ready, wanted) = ready_part(ready, wanted);
            Reply::Pending { chunks: m.into_iter().map(|m| m.chunk as f64).collect(), reached_bytes, ready, wanted }
        }
        Err(EvalError::Busy { reached_bits }) => {
            let (ready, wanted) = ready_part(ready, wanted);
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

//! How fields connect: the graph, the diagram of types, and the census of what was read.

use crate::*;

/// One field of a subtree, as much of it as an arrow needs. No value: what a
/// field says is what makes reading it expensive, and nothing a graph draws
/// shows it.
#[derive(Serialize)]
pub(crate) struct GraphNodeDto {
    pub(crate) path: Vec<f64>,
    pub(crate) name: String,
    /// The resolved type said coarsely, for a view that groups or colours by
    /// it: "u16", "u32", "f32", "bytes", "str", "struct", "array", "repeat"
    /// and the rest. See `qubero_core::eval::kind_of`.
    pub(crate) kind: String,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    /// Index into the node list, or -1 for the node the graph was asked for.
    pub(crate) parent: f64,
    pub(crate) child_count: f64,
    /// True when this node has children the walk stopped short of.
    pub(crate) truncated: bool,
}

/// One field deciding something about another, as an arrow between two nodes.
#[derive(Serialize)]
pub(crate) struct GraphEdgeDto {
    /// Index into the node list: the field that decided.
    pub(crate) from: f64,
    /// Index into the node list: the field it decided about.
    pub(crate) to: f64,
    /// "length" | "count" | "type" | "position" | "value" | "name" | "width" | "points"
    pub(crate) role: &'static str,
}

/// The subtree under one node, and what its fields decide about each other.
#[derive(Serialize)]
pub(crate) struct GraphDto {
    pub(crate) nodes: Vec<GraphNodeDto>,
    pub(crate) edges: Vec<GraphEdgeDto>,
    /// How many nodes under the one asked about were left out, as far as is
    /// known.
    pub(crate) omitted: f64,
}

/// One row of a diagram box: one field of a type, or one case of a switch.
#[derive(Serialize)]
pub(crate) struct DiagramRowDto {
    pub(crate) name: String,
    /// The type as the listing's type column writes it.
    pub(crate) type_text: String,
    /// How long the field runs, or the expression that decides it. Empty when
    /// only reading a file settles it.
    pub(crate) size_text: String,
    /// Where it starts inside its own type, or the address it reads its
    /// contents at. Empty when neither is fixed by the template.
    pub(crate) pos_text: String,
    /// True when the field holds a list of elements. See `diagram::Row::list`.
    pub(crate) list: bool,
    /// The word the listing gives a field of this type, so the same field is
    /// the same colour in both: "uint", "str", "magic", "composite" and the
    /// rest. See `qubero_core::eval::value_kind`.
    pub(crate) kind: &'static str,
}

/// One type of the format, and its fields.
#[derive(Serialize)]
pub(crate) struct DiagramBoxDto {
    /// What the type is called: the structure's own name in the template.
    pub(crate) name: String,
    /// Where the walk first reached it, for the reader who wants to know how
    /// they would get there.
    pub(crate) path: String,
    /// What tells this type from every other. `diagram_census` counts the open
    /// file's nodes by the same key, so a view can say how many of this box the
    /// file holds.
    pub(crate) key: String,
    /// "seq" | "instances" | "switch"
    pub(crate) kind: &'static str,
    /// The type this one was written inside, for a box the template gave no
    /// name of its own. Absent for a named type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent: Option<String>,
    pub(crate) rows: Vec<DiagramRowDto>,
}

/// One connection, from the row that decides to the row it decides about.
#[derive(Serialize)]
pub(crate) struct DiagramEdgeDto {
    /// Box index and row index: where the edge leaves.
    pub(crate) from: (f64, f64),
    /// Box index where it lands.
    pub(crate) to: f64,
    /// Row index in that box, absent for an edge to the box as a whole, which
    /// is what naming a type is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) to_row: Option<f64>,
    /// "length" | "count" | "type" | "position" | "value" | "name" | "width" | "condition" | "case"
    pub(crate) role: &'static str,
    /// The expression the edge stands for, as the template writes it. Empty for
    /// a declaration rather than an expression.
    pub(crate) label: String,
}

/// The format as boxes and arrows, read off the template rather than a file.
#[derive(Serialize)]
pub(crate) struct DiagramDto {
    pub(crate) types: Vec<DiagramBoxDto>,
    pub(crate) edges: Vec<DiagramEdgeDto>,
    /// Named types of the template with no box here.
    pub(crate) omitted: f64,
}

pub(crate) fn diagram_dto(d: Diagram) -> DiagramDto {
    DiagramDto {
        types: d
            .types
            .into_iter()
            .map(|b| DiagramBoxDto {
                name: b.name,
                path: b.path,
                key: b.key,
                kind: b.kind.as_str(),
                parent: b.parent,
                rows: b
                    .rows
                    .into_iter()
                    .map(|r| DiagramRowDto {
                        name: r.name,
                        type_text: r.type_text,
                        size_text: r.size_text,
                        pos_text: r.pos_text,
                        list: r.list,
                        kind: r.kind,
                    })
                    .collect(),
            })
            .collect(),
        edges: d
            .edges
            .into_iter()
            .map(|e| DiagramEdgeDto {
                from: (e.from.0 as f64, e.from.1 as f64),
                to: e.to.0 as f64,
                to_row: e.to.1.map(|r| r as f64),
                role: e.role.as_str(),
                label: e.label,
            })
            .collect(),
        omitted: f64::from(d.omitted),
    }
}

/// How many of one diagram box the open file holds, and where the first is.
#[derive(Serialize)]
pub(crate) struct BoxCountDto {
    /// Matches `DiagramBoxDto::key`.
    pub(crate) key: String,
    pub(crate) count: f64,
    /// Child indices from the root, and which reading they are in: 0 is the
    /// file, anything else an unpacked stream.
    pub(crate) first_path: Vec<f64>,
    pub(crate) space: f64,
}

/// The same for one row of one box.
#[derive(Serialize)]
pub(crate) struct RowCountDto {
    pub(crate) key: String,
    pub(crate) row: f64,
    pub(crate) count: f64,
    pub(crate) first_path: Vec<f64>,
    pub(crate) space: f64,
}

/// What the open file holds, against what the format can hold.
#[derive(Serialize)]
pub(crate) struct CensusDto {
    pub(crate) boxes: Vec<BoxCountDto>,
    pub(crate) rows: Vec<RowCountDto>,
    /// How many nodes the walk has looked at.
    pub(crate) walked: f64,
    /// `done` when every node was counted. Otherwise every count is a floor,
    /// and this says what to do about it: `working` asks again at once,
    /// `waiting` asks again when the bytes it asked for land, and `capped`
    /// asks again only with a higher limit.
    pub(crate) state: &'static str,
}

pub(crate) fn census_dto(c: Census) -> CensusDto {
    CensusDto {
        boxes: c
            .boxes
            .into_iter()
            .map(|b| BoxCountDto {
                key: b.key,
                count: b.count as f64,
                first_path: b.first_path.into_iter().map(|x| x as f64).collect(),
                space: f64::from(b.space),
            })
            .collect(),
        rows: c
            .rows
            .into_iter()
            .map(|r| RowCountDto {
                key: r.key,
                row: r.row as f64,
                count: r.count as f64,
                first_path: r.first_path.into_iter().map(|x| x as f64).collect(),
                space: f64::from(r.space),
            })
            .collect(),
        walked: c.walked as f64,
        state: match c.state {
            CensusState::Done => "done",
            CensusState::Working => "working",
            CensusState::Waiting => "waiting",
            CensusState::Capped => "capped",
        },
    }
}

/// The graph as the host reads it. The one thing translated rather than copied
/// is the root's parent: the core says `usize::MAX`, which as a JSON number is
/// sixteen digits of nonsense, and -1 is what every other absent index here is.
pub(crate) fn graph_dto(g: Graph) -> GraphDto {
    GraphDto {
        nodes: g
            .nodes
            .into_iter()
            .map(|n| GraphNodeDto {
                path: n.path.into_iter().map(|x| x as f64).collect(),
                name: n.name,
                kind: n.kind,
                offset_bits: n.offset_bits as f64,
                size_bits: n.size_bits as f64,
                parent: if n.parent == NO_PARENT { -1.0 } else { n.parent as f64 },
                child_count: n.child_count as f64,
                truncated: n.truncated,
            })
            .collect(),
        edges: g
            .edges
            .into_iter()
            .map(|e| GraphEdgeDto { from: e.from as f64, to: e.to as f64, role: e.role })
            .collect(),
        omitted: g.omitted as f64,
    }
}

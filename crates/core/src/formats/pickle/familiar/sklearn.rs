//! The callables scikit-learn writes, and what each argument has to be.
//!
//! An estimator itself needs none of these: it is a class under `sklearn.`,
//! `cls.__new__(cls)`, and a dictionary of attributes handed over by BUILD,
//! which is the plain object production in [`object`](super::object). What is
//! here is the rest of what a fitted model turns out to hold, which is three
//! things:
//!
//! * **A tree of nodes**, which is a `sklearn.tree._tree.Tree` made from the
//!   shape it was fitted to and filled by the BUILD after it.
//! * **A Cython object made the long way round.** A class written in Cython
//!   has no `__new__` a pickle can call through NEWOBJ, so the module keeps a
//!   one-line function that does it: `newObj(cls)` is `cls.__new__(cls)`, and
//!   the BUILD after it is the state, exactly as for an estimator. A KD-tree,
//!   a ball tree and a distance metric all arrive that way.
//! * **A loss function**, which is a Cython class whose `__reduce__` hands the
//!   class back with the one number it was configured with, or with nothing.
//!
//! The safety line does not move. Each callable is named by its whole dotted
//! path and checked against the argument shape the library writes it with;
//! nothing is run, and a REDUCE of any other global under `sklearn` is still a
//! non-match. The lists were read off scikit-learn 1.9 and every entry was
//! measured against the bytes that release writes.

use super::forms::{Args, Reduce, Via};
use super::{Kind, Shape, Value};
use super::cursor::Cursor;

/// The package scikit-learn's classes come from, named by the plain form and
/// by the joblib one over it.
pub(super) const SKLEARN_CLASSES: &[&str] = &["sklearn"];

/// `cls.__new__(cls)` under another name: one argument, the class, which
/// reached the reader through the same `may_name` check every other class
/// goes through and is therefore one under `sklearn.`.
const NEW_OBJECT: Reduce = Reduce {
    via: Via::Global,
    path: "",
    what: Shape::Object,
    names: &["type"],
    args: Args::Fixed,
    shape: |_c, args| matches!(args[0].kind, Kind::Class { .. }).then_some(()),
};

/// A loss configured with nothing at all.
const LOSS: Reduce = Reduce { via: Via::Global, path: "", what: Shape::Object, names: &[], args: Args::Fixed, shape: |_c, _args| Some(()) };

/// A loss configured with one number, under the name that number goes by.
const LOSS_OF: Reduce = Reduce { names: &["parameter"], shape: one_number, ..LOSS };

/// The one argument a configured loss takes, which is always a float even
/// where the setting reads as a whole number.
fn one_number(_c: &Cursor, args: &[Value]) -> Option<()> {
    matches!(args[0].kind, Kind::Float { .. }).then_some(())
}

pub(super) const SKLEARN_CALLS: &[Reduce] = &[
    // A decision tree's array of nodes lives in a `Tree`, which is
    // constructed from how many features, classes and outputs it was fitted
    // on and handed its arrays by the BUILD after it.
    Reduce {
        via: Via::Global,
        path: "sklearn.tree._tree.Tree",
        what: Shape::Object,
        names: &["n_features", "n_classes", "n_outputs"],
        args: Args::Fixed,
        shape: |_c, args| {
            matches!(
                (&args[0].kind, &args[1].kind, &args[2].kind),
                (Kind::Int { .. }, Kind::Array { .. }, Kind::Int { .. })
            )
            .then_some(())
        },
    },
    // The three modules that keep a `newObj` of their own. Each is the same
    // one line of Cython, and a nearest-neighbour model holds a tree built by
    // one of the first two and a distance metric built by the third.
    Reduce { path: "sklearn.neighbors._kd_tree.newObj", ..NEW_OBJECT },
    Reduce { path: "sklearn.neighbors._ball_tree.newObj", ..NEW_OBJECT },
    Reduce { path: "sklearn.metrics._dist_metrics.newObj", ..NEW_OBJECT },
    // The losses `sklearn._loss` is built out of, which every boosted model
    // and every linear model with a loss of its own holds. Four take the one
    // number they were configured with and the rest take nothing.
    Reduce { path: "sklearn._loss._loss.CyHalfSquaredError", ..LOSS },
    Reduce { path: "sklearn._loss._loss.CyAbsoluteError", ..LOSS },
    Reduce { path: "sklearn._loss._loss.CyHalfPoissonLoss", ..LOSS },
    Reduce { path: "sklearn._loss._loss.CyHalfGammaLoss", ..LOSS },
    Reduce { path: "sklearn._loss._loss.CyHalfBinomialLoss", ..LOSS },
    Reduce { path: "sklearn._loss._loss.CyExponentialLoss", ..LOSS },
    Reduce { path: "sklearn._loss._loss.CyPinballLoss", names: &["quantile"], ..LOSS_OF },
    Reduce { path: "sklearn._loss._loss.CyHuberLoss", names: &["delta"], ..LOSS_OF },
    Reduce { path: "sklearn._loss._loss.CyHalfTweedieLoss", names: &["power"], ..LOSS_OF },
    Reduce { path: "sklearn._loss._loss.CyHalfTweedieLossIdentity", names: &["power"], ..LOSS_OF },
    // The losses the stochastic gradient descent models keep, which are a
    // second set written before `sklearn._loss` existed and still in use.
    Reduce { path: "sklearn.linear_model._sgd_fast.ModifiedHuber", ..LOSS },
    Reduce { path: "sklearn.linear_model._sgd_fast.Hinge", names: &["threshold"], ..LOSS_OF },
    Reduce { path: "sklearn.linear_model._sgd_fast.SquaredHinge", names: &["threshold"], ..LOSS_OF },
    Reduce { path: "sklearn.linear_model._sgd_fast.EpsilonInsensitive", names: &["epsilon"], ..LOSS_OF },
    Reduce { path: "sklearn.linear_model._sgd_fast.SquaredEpsilonInsensitive", names: &["epsilon"], ..LOSS_OF },
    // What `random_state` holds after a fit, which is NumPy's rather than
    // scikit-learn's, so the rows are shared with the table that declares
    // them rather than copied here.
    super::numpy::BIT_GENERATOR_CTOR,
    super::numpy::RANDOM_STATE_CTOR,
    super::numpy::GENERATOR_CTOR,
    super::numpy::SEED_SEQUENCE_CTOR,
];

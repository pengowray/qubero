# FPF fixtures

`numpy-f32-matrix.pickle` is copied unchanged from the existing Qubero sample
collection's `pickle/proto4-numpy-array.pickle`.

Expected object: a dictionary with key `weights` and a 4-by-6 C-order,
little-endian float32 NumPy array containing 0 through 23. Protocol: 4.
Producer Python/NumPy versions and generation command: unknown.

`joblib-array-small.joblib` and `joblib-three-arrays.joblib` are copied
unchanged from the collection's `joblib/array-small.joblib` and
`joblib/list-of-arrays.joblib`, written by `tools/make_torch_joblib_samples.py`
with joblib 1.6, NumPy 2.5 and Python 3.12.

Expected objects: `numpy.arange(6, dtype="float64").reshape(2, 3)` on its own,
and a list of `arange(4, "int8")`, `arange(4, "int8")` and
`arange(3, "float32")`. Protocol: 4. Each array's numbers are in the file
after the wrapper that describes them, padded to sixteen bytes.

The Rust tests inspect the committed bytes directly. Python is not required.
These fixtures establish observed instruction forms, not broad NumPy, joblib
or producer-version compatibility.

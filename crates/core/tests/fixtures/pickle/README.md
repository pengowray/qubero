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

`torch-v0.4-parameter-data.pkl` and `torch-v0.4-size-data.pkl` are the data
pickles of `torch-parameter.pt` and `torch-size-device-dtype.pt` from the
container matrix run of torch 0.4.1, NumPy 1.19 and Python 3.6, cut out of
those legacy files at the fourth pickle's own bounds.

Expected objects: `{'p': Parameter(arange(6).reshape(2, 3), float32,
requires_grad=True)}`, and `{'size': torch.Size((2, 3)), 'device':
torch.device('cpu'), 'dtype': torch.float32}`. Protocol: 2. They are here for
the two spellings that release used and no later one does: a tensor whose
backward hooks are `None`, and a `torch.Size` that NEWOBJ closes.

The Rust tests inspect the committed bytes directly. Python is not required.
These fixtures establish observed instruction forms, not broad NumPy, joblib
or producer-version compatibility.

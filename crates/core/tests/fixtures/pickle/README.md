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

`torch-v1.5-protocol4-legacy.pt` is `torch-protocol4.pt` from the same run
with torch 1.5.1, NumPy 1.21 and Python 3.7: `torch.save(state_dict, path,
pickle_protocol=4)`, which that release wrote as a legacy file.

Expected object: an `OrderedDict` of `layer.weight` (`arange(12).reshape(3,
4)`, float32), `layer.bias` (three zeroes) and `steps` (the int64 `7`).
Protocol: 4. It is here for the other opener the legacy format has and for
the storage keys torch named by buffer address before 1.6.

`torch-v1.0-whole-module-data.pkl` is the data pickle of
`torch.save(torch.nn.Linear(4, 3), path)` from the same run with torch 1.0.1,
NumPy 1.19 and Python 3.6.

Expected object: a `Linear` whose class arrives through the persistent id
`('module', cls, source_file, source)` and whose state holds `_backend`,
`_parameters` (weight `arange(12).reshape(3, 4)` float32 and a bias of three),
eight empty `OrderedDict`s, `training`, `in_features` and `out_features`.
Protocol: 2. It is here for the class source a legacy save carries and for the
backend call torch 1.1 dropped.

`joblib-v0.9-npy-files.joblib` is `joblib-dict-of-arrays.joblib` from the
container matrix run of joblib 0.9.4, NumPy 1.19 and Python 3.6.

Expected object: `{"name": "bundle", "weights": arange(24, float64).reshape(4,
6), "bias": zeros(4, float32), "labels": ["a", "b", "c"], "steps": 7}`.
Protocol: 3. Before 0.10 joblib wrote each array as a `.npy` file beside the
pickle, so this file holds two wrappers naming
`joblib-dict-of-arrays.joblib_01.npy` and `_02.npy` and none of the numbers.
The two `.npy` files are in the collection under `joblib/v0.9-npy-files/`.

The Rust tests inspect the committed bytes directly. Python is not required.
These fixtures establish observed instruction forms, not broad NumPy, joblib
or producer-version compatibility.

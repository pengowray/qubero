# FPF fixture

`numpy-f32-matrix.pickle` is copied unchanged from the existing Qubero sample
collection's `pickle/proto4-numpy-array.pickle`.

Expected object: a dictionary with key `weights` and a 4-by-6 C-order,
little-endian float32 NumPy array containing 0 through 23. Protocol: 4.
Producer Python/NumPy versions and generation command: unknown.

The Rust tests inspect the committed bytes directly. Python is not required.
This fixture establishes one observed instruction form, not broad NumPy or
producer-version compatibility.

# Rules
- All fields in structures should be private.
- Only the fields accessed should have getters
- All functions should be private.
- Everything should be as private as possible.
- Do not use pub(crate) or pub(super)
- Do not make extra allocations using clone, to_string, to_owned, to_vec and so on, leverage lifetimes.
- Do not overuse neither Arc, nor Mutex.
- For async functions, privilege returning impl Future<...>.

use armature_proc_macro::Validate;

// A non-`usize` length bound is a type error, not an `as` cast that would
// turn `-1` into `usize::MAX`.
const NEG: i64 = -1;

#[derive(Validate)]
struct LengthBoundNotUsize {
    #[validate(length(max = NEG))]
    a: String,
}

fn main() {}

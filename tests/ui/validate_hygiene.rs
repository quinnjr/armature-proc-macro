use armature_proc_macro::Validate;

// User expressions cannot see the derive's generated locals: each name below
// exists inside the generated `validate()`, but is out of reach of the
// user-written bound or closure, so it does not resolve. Without hygiene
// these would compile and read (or clear) the generated state.
#[derive(Validate)]
struct ReadsValue {
    #[validate(range(max = __armature_value))]
    n: u64,
}

#[derive(Validate)]
struct ClearsErrors {
    #[validate(custom = "|_: &String| -> Result<(), armature_validation::ValidationError> { __armature_errors.clear(); Ok(()) }")]
    s: String,
}

fn main() {}

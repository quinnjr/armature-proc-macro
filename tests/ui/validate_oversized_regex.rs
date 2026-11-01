use armature_proc_macro::Validate;

// Parses, but is over the compiled-size limit the runtime enforces (the
// message is the `regex` crate's own wording).
#[derive(Validate)]
struct Oversized {
    #[validate(regex = r"\w{300}")]
    word: String,
}

fn main() {}

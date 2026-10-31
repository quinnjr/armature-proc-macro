use armature_proc_macro::Validate;

#[derive(Validate)]
struct UnknownRegexKey {
    #[validate(regex(patern = "[a-z]+"))]
    a: String,
}

#[derive(Validate)]
struct DuplicateBound {
    #[validate(range(min = 10, min = 0))]
    a: u8,
}

#[derive(Validate)]
struct InvertedBounds {
    #[validate(length(min = 5, max = 2))]
    a: String,
}

#[derive(Validate)]
struct EmptyValidate {
    #[validate()]
    a: String,
}

const NEG: i64 = -1;

#[derive(Validate)]
struct LengthBoundNotUsize {
    #[validate(length(max = NEG))]
    a: String,
}

fn main() {}

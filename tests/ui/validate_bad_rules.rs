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

#[derive(Validate)]
struct DuplicateRule {
    #[validate(email)]
    #[validate(email)]
    a: String,
}

#[derive(Validate)]
struct BoundlessRange {
    #[validate(range())]
    a: u8,
}

#[derive(Validate)]
struct BoundlessLength {
    #[validate(length())]
    a: String,
}

#[derive(Validate)]
struct NestedOptionRequired {
    #[validate(required)]
    a: Option<Option<String>>,
}

#[derive(Validate)]
struct NestedOptionBehindPointer {
    #[validate(required)]
    a: Option<Box<Option<String>>>,
}

fn main() {}

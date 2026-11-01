use armature_proc_macro::Validate;

// `required` on a generic `Option<T>` needs `T: AsRef<str>`: without the
// bound it fails to compile instead of silently skipping the emptiness check.
#[derive(Validate)]
struct Named<T> {
    #[validate(required)]
    name: Option<T>,
}

// Likewise a generic wrapper the derive cannot see through: it must itself
// be `AsRef<str>`, not silently mean "is `Some`".
struct Wrapper<T>(T);

#[derive(Validate)]
struct Wrapped<T> {
    #[validate(required)]
    name: Option<Wrapper<T>>,
}

// A type macro nested in another type may hide the parameter: fail closed.
macro_rules! id {
    ($t:ty) => {
        $t
    };
}

struct N<T>(T);

#[derive(Validate)]
struct HiddenByMacro<T> {
    #[validate(required)]
    name: Option<N<id!(T)>>,
}

// A where-clause type predicate forces the isolated check, which cannot
// name `Self`: an inner `Box<Self>` must then be `AsRef<str>`.
#[derive(Validate)]
struct SelfUnderPredicate<'a>
where
    String: Sized,
{
    #[validate(required)]
    parent: Option<Box<Self>>,
    tag: &'a str,
}

// A container name through another module is not trusted as never a
// string (`m::Vec` could deref to one, and a where-clause hide it).
mod m {
    pub struct Vec<T>(pub T);
}

#[derive(Validate)]
struct LookalikeContainer<T>
where
    m::Vec<T>: Sized,
{
    #[validate(required)]
    v: Option<m::Vec<T>>,
}

// Under a where-clause type predicate, a named container of `Self` fails
// closed (only tuples, arrays and slices are exempt).
#[derive(Validate)]
struct SelfSetUnderPredicate<'a>
where
    String: Sized,
{
    #[validate(required)]
    kids: Option<std::collections::BTreeSet<Self>>,
    tag: &'a str,
}

// A pointer look-alike in scope under a standard name, over a tuple of
// `Self`, with a where-clause that would hide its `Target`: the derive's
// proof that the layers lead to the tuple fails to compile.
mod lookalike {
    pub struct Rc<T>(pub String, pub std::marker::PhantomData<T>);
    impl<T> std::ops::Deref for Rc<T> {
        type Target = String;
        fn deref(&self) -> &String {
            &self.0
        }
    }
}
use lookalike::Rc;

#[derive(Validate)]
struct ShadowedPointer<'a>
where
    Rc<(ShadowedPointer<'a>,)>: std::ops::Deref,
{
    #[validate(required)]
    f: Option<Rc<(Self,)>>,
    tag: &'a str,
}

fn main() {}

//! Behavioral tests for the `#[use_guard]` / `#[guard]` decorators.

use armature_core::guard::{Guard, GuardContext};
use armature_core::middleware::{Middleware, Next};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{controller, guard, middleware, module, routes, use_guard};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

mod support;
use support::{build_router, get_request};

#[derive(Default)]
struct AllowGuard;

#[async_trait::async_trait]
impl Guard for AllowGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(true)
    }
}

#[derive(Default)]
struct DenyGuard;

#[async_trait::async_trait]
impl Guard for DenyGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(false)
    }
}

/// A guard constructed with an argument, exercising the instance-expression form.
struct FlagGuard {
    allow: bool,
}

impl FlagGuard {
    fn new(allow: bool) -> Self {
        Self { allow }
    }
}

#[async_trait::async_trait]
impl Guard for FlagGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(self.allow)
    }
}

fn request() -> HttpRequest {
    HttpRequest::from_parts(
        "GET",
        "/protected".to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        HashMap::new(),
    )
}

// Type-based guard via `#[use_guard]`. The handler parameter is named `req`
// and referenced in the body: the wrapper must preserve that binding.
#[use_guard(AllowGuard)]
async fn allowed(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = req.method.clone();
    Ok(HttpResponse::ok())
}

#[use_guard(DenyGuard)]
async fn denied(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn passing_guard_runs_handler() {
    let resp = allowed(request()).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn failing_guard_blocks_handler() {
    let err = denied(request()).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)));
}

// Instance-based guard via `#[guard(expr)]`.
#[guard(FlagGuard::new(true))]
async fn allowed_instance(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[guard(FlagGuard::new(false))]
async fn denied_instance(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn instance_guard_runs_handler() {
    let resp = allowed_instance(request()).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn instance_guard_can_block() {
    let err = denied_instance(request()).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)));
}

// ---------------------------------------------------------------------------
// Controller-STRUCT-level `#[guard(...)]` enforcement.
//
// A guard attached to the controller struct must protect *every* route
// registered for that controller through the module route registrar — not
// just handlers carrying their own `#[use_guard]`. These tests drive a real
// request through the generated registrar + `Router`.
// ---------------------------------------------------------------------------

#[controller("/deny")]
#[guard(DenyGuard)]
#[derive(Default)]
struct DeniedController;

#[routes]
impl DeniedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [DeniedController])]
#[derive(Default)]
struct DeniedModule;

#[controller("/allow")]
#[guard(AllowGuard)]
#[derive(Default)]
struct AllowedController;

#[routes]
impl AllowedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [AllowedController])]
#[derive(Default)]
struct AllowedModule;

#[tokio::test]
async fn controller_struct_guard_denies_route() {
    let router = build_router::<DeniedModule>();
    let resp = router.route(get_request("/deny/thing")).await.unwrap();
    assert_eq!(
        resp.status, 403,
        "controller-struct #[guard] must reject the route (got {})",
        resp.status
    );
}

#[tokio::test]
async fn controller_struct_guard_allows_route() {
    let router = build_router::<AllowedModule>();
    let resp = router.route(get_request("/allow/thing")).await.unwrap();
    assert_eq!(
        resp.status, 200,
        "AllowGuard must let the route through (got {})",
        resp.status
    );
}

// ---------------------------------------------------------------------------
// Guards are built ONCE, at route-registration time — never on the request
// path — and a denying guard short-circuits the ones after it.
// ---------------------------------------------------------------------------

static FIRST_DENY_CONSTRUCTED: AtomicUsize = AtomicUsize::new(0);
static SECOND_CONSTRUCTED: AtomicUsize = AtomicUsize::new(0);
static SECOND_ACTIVATED: AtomicUsize = AtomicUsize::new(0);

struct CountingDenyGuard;

impl Default for CountingDenyGuard {
    fn default() -> Self {
        FIRST_DENY_CONSTRUCTED.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

#[async_trait::async_trait]
impl Guard for CountingDenyGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(false)
    }
}

/// Must never be *evaluated* (the guard before it denies) and must be
/// *constructed* exactly once, at registration.
struct NeverReachedGuard;

impl Default for NeverReachedGuard {
    fn default() -> Self {
        SECOND_CONSTRUCTED.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

#[async_trait::async_trait]
impl Guard for NeverReachedGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        SECOND_ACTIVATED.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

#[controller("/multi")]
#[guard(CountingDenyGuard, NeverReachedGuard)]
#[derive(Default)]
struct MultiGuardController;

#[routes]
impl MultiGuardController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [MultiGuardController])]
#[derive(Default)]
struct MultiGuardModule;

#[tokio::test]
async fn controller_struct_guards_are_built_once_and_short_circuit() {
    let router = build_router::<MultiGuardModule>();

    // Registration builds each guard exactly once, up front.
    assert_eq!(
        FIRST_DENY_CONSTRUCTED.load(Ordering::SeqCst),
        1,
        "guard must be constructed once at route registration"
    );
    assert_eq!(
        SECOND_CONSTRUCTED.load(Ordering::SeqCst),
        1,
        "guard must be constructed once at route registration"
    );

    for _ in 0..3 {
        let resp = router.route(get_request("/multi/thing")).await.unwrap();
        assert_eq!(resp.status, 403, "denying guard must reject the route");
    }

    // Three requests later the counts are unchanged: no guard is constructed
    // on the request path.
    assert_eq!(
        FIRST_DENY_CONSTRUCTED.load(Ordering::SeqCst),
        1,
        "guards must not be rebuilt per request"
    );
    assert_eq!(
        SECOND_CONSTRUCTED.load(Ordering::SeqCst),
        1,
        "guards must not be rebuilt per request"
    );
    assert_eq!(
        SECOND_ACTIVATED.load(Ordering::SeqCst),
        0,
        "a denying guard must short-circuit the guards after it"
    );
}

// ---------------------------------------------------------------------------
// A guard's `Err` propagates instead of collapsing into the generic 403.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ExplodingGuard;

#[async_trait::async_trait]
impl Guard for ExplodingGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Err(Error::Internal("guard exploded".to_string()))
    }
}

#[controller("/exploding")]
#[guard(ExplodingGuard)]
#[derive(Default)]
struct ExplodingController;

#[routes]
impl ExplodingController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [ExplodingController])]
#[derive(Default)]
struct ExplodingModule;

#[tokio::test]
async fn controller_struct_guard_error_propagates() {
    let router = build_router::<ExplodingModule>();
    let result = router.route(get_request("/exploding/thing")).await;

    match result {
        Err(Error::Internal(msg)) => assert_eq!(msg, "guard exploded"),
        Err(other) => panic!("expected the guard's own error, got {other:?}"),
        Ok(resp) => panic!(
            "guard error must propagate, not become a {} response with body {:?}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        ),
    }
}

// ---------------------------------------------------------------------------
// Guard + middleware on one controller must not leak onto a plain,
// unannotated controller registered in the same module.
// ---------------------------------------------------------------------------

struct GuardStampMiddleware;

#[async_trait::async_trait]
impl Middleware for GuardStampMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let mut resp = next(req).await?;
        resp.headers
            .insert("X-Guard-Stamp".to_string(), "1".to_string());
        Ok(resp)
    }
}

#[controller("/mixed")]
#[guard(DenyGuard)]
#[middleware(GuardStampMiddleware)]
#[derive(Default)]
struct MixedController;

#[routes]
impl MixedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[controller("/plain")]
#[derive(Default)]
struct PlainController;

#[routes]
impl PlainController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [MixedController, PlainController])]
#[derive(Default)]
struct MixedModule;

#[tokio::test]
async fn annotated_controller_does_not_affect_sibling_controller() {
    let router = build_router::<MixedModule>();

    let guarded = router.route(get_request("/mixed/thing")).await.unwrap();
    assert_eq!(
        guarded.status, 403,
        "the annotated controller's guard must still deny"
    );
    assert_eq!(
        guarded.headers.get("X-Guard-Stamp"),
        Some(&"1".to_string()),
        "middleware wraps the guards (Nest order), so it sees the guard's 403"
    );

    let plain = router.route(get_request("/plain/thing")).await.unwrap();
    assert_eq!(
        plain.status, 200,
        "the unannotated controller must be served unguarded"
    );
    assert_eq!(
        plain.headers.get("X-Guard-Stamp"),
        None,
        "the sibling controller's middleware must not wrap this route"
    );
}

// ---------------------------------------------------------------------------
// Ordering: controller middleware runs BEFORE controller guards (NestJS order
// middleware -> guards -> handler), so an authentication middleware can attach
// a verified identity that a guard then checks. With the old guards-first
// order this route was unreachable: the guard never saw the identity.
// ---------------------------------------------------------------------------

/// Stand-in for a verified identity attached by an authentication middleware
/// (what `JwtAuthMiddleware` does with `UserContext`).
#[derive(Clone)]
struct VerifiedUser {
    roles: Vec<&'static str>,
}

/// Authentication middleware: "verifies" the request and attaches the identity.
struct AttachIdentityMiddleware;

#[async_trait::async_trait]
impl Middleware for AttachIdentityMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        req.insert_extension(VerifiedUser {
            roles: vec!["admin"],
        });
        next(req).await
    }
}

/// Role guard that fails closed without a verified identity extension.
#[derive(Default)]
struct RequireAdminGuard;

#[async_trait::async_trait]
impl Guard for RequireAdminGuard {
    async fn can_activate(&self, ctx: &GuardContext) -> Result<bool, Error> {
        Ok(ctx
            .request
            .extension::<VerifiedUser>()
            .is_some_and(|u| u.roles.contains(&"admin")))
    }
}

static ORDER_LOG: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// Records when it runs relative to the guard and the handler.
struct LoggingMiddleware;

#[async_trait::async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        ORDER_LOG.lock().unwrap().push("middleware");
        next(req).await
    }
}

#[derive(Default)]
struct LoggingGuard;

#[async_trait::async_trait]
impl Guard for LoggingGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        ORDER_LOG.lock().unwrap().push("guard");
        Ok(true)
    }
}

#[controller("/authed")]
#[guard(RequireAdminGuard)]
#[middleware(AttachIdentityMiddleware)]
#[derive(Default)]
struct AuthedController;

#[routes]
impl AuthedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[controller("/unauthed")]
#[guard(RequireAdminGuard)]
#[derive(Default)]
struct UnauthedController;

#[routes]
impl UnauthedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }
}

#[controller("/ordered")]
#[guard(LoggingGuard)]
#[middleware(LoggingMiddleware)]
#[derive(Default)]
struct OrderedController;

#[routes]
impl OrderedController {
    #[get("/thing")]
    async fn thing() -> Result<HttpResponse, Error> {
        ORDER_LOG.lock().unwrap().push("handler");
        Ok(HttpResponse::ok())
    }
}

#[module(controllers: [AuthedController, UnauthedController, OrderedController])]
#[derive(Default)]
struct OrderingModule;

#[tokio::test]
async fn controller_middleware_populates_identity_before_guard_runs() {
    let router = build_router::<OrderingModule>();
    let resp = router.route(get_request("/authed/thing")).await.unwrap();
    assert_eq!(
        resp.status, 200,
        "the guard must see the identity the controller middleware attached"
    );
}

#[tokio::test]
async fn identity_guard_fails_closed_without_auth_middleware() {
    let router = build_router::<OrderingModule>();
    let resp = router.route(get_request("/unauthed/thing")).await.unwrap();
    assert_eq!(resp.status, 403);
}

#[tokio::test]
async fn controller_pipeline_runs_middleware_then_guard_then_handler() {
    let router = build_router::<OrderingModule>();
    ORDER_LOG.lock().unwrap().clear();
    let resp = router.route(get_request("/ordered/thing")).await.unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(
        *ORDER_LOG.lock().unwrap(),
        vec!["middleware", "guard", "handler"]
    );
}

// ---------------------------------------------------------------------------
// The no-argument forms must re-emit the handler untouched. They used to emit
// the function's attributes and visibility *in front of* the whole `ItemFn` —
// which already carries both — producing duplicated attributes and
// `pub pub async fn`.
// ---------------------------------------------------------------------------

#[guard()]
#[inline]
pub async fn unguarded(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[use_guard()]
pub async fn also_unguarded(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn empty_guard_lists_leave_the_handler_intact() {
    assert_eq!(unguarded(request()).await.unwrap().status, 200);
    assert_eq!(also_unguarded(request()).await.unwrap().status, 200);
}

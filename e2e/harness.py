"""Real-stack checks; unavailable capabilities remain explicit non-passing evidence."""

import importlib.util
import json
import re
from concurrent.futures import ThreadPoolExecutor
from urllib.error import URLError

from e2e.catalog import FLOWS
from e2e.fixtures import PASSWORD, create_member, mail_token, new_signup
from e2e.http_client import Client, require_loopback

BY_ID = {flow.id: flow for flow in FLOWS}
STATIC = {"C01", "C18", "C36", "C40"}
PUBLIC = {"C02", "C03", "C04", "C05", "C06", "C20", "C29"}
POST = {"C02", "C03", "C04", "C05", "C06", "C09", "C10", "C20", "C21",
        "C23", "C24", "C25", "C27", "A04", "A06"}
MISSING_FIELDS = {"C02": "email", "C03": "token", "C04": "email", "C05": "email"}


class CheckFailure(Exception):
    def __init__(self, result, gap, detail):
        super().__init__(f"{gap}: {detail}")
        self.result = result
        self.gap = gap


def check(condition, gap, detail):
    if not condition:
        raise CheckFailure("FAIL", gap, detail)


class Session:
    def __init__(self, args):
        self.args = args
        for origin in (args.origin, args.site_open, args.site_closed):
            require_loopback(origin)
        self.client = Client(args.origin)
        self.member = None
        self.member_error = None
        self.campaign_id = None
        self.email = None
        self.playwright = None
        self.browser = None
        self.context = None
        self.page = None
        self.observation = ""
        self.browser_errors = []
        self.capture_error = None
        self.screens = args.output.parent / "screens"
        self.screens.mkdir(parents=True, exist_ok=True)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close_page()
        if self.browser:
            self.browser.close()
        if self.playwright:
            self.playwright.stop()

    def close_page(self):
        if self.context:
            self.context.close()
        self.page = self.context = None

    def begin(self, _case):
        self.close_page()
        self.observation = ""
        self.browser_errors = []
        self.capture_error = None

    def blocked(self, detail, gap="FLOW-FIXTURES"):
        raise CheckFailure("BLOCKED", gap, detail)

    def require_member(self):
        if self.member_error:
            self.blocked(self.member_error)
        if self.member is None:
            try:
                self.member, self.email = create_member(self.args.origin, self.args.outbox)
            except (AssertionError, OSError, URLError) as error:
                self.member_error = str(error)
                self.blocked(self.member_error)
        return self.member

    def client_for(self, flow, anonymous=False):
        client = self.client if anonymous or flow.id in PUBLIC else self.require_member()
        client.session()
        return client

    def endpoint(self, flow, foreign=False):
        path = flow.endpoint
        if "{session}" in path:
            if not foreign:
                self.blocked("No admitted session fixture; start is owned by RUNTIMEPLACE")
            path = path.replace("{session}", "f0" * 16)
        if "{campaign}" in path:
            value = "f1" * 16 if foreign else self.require_campaign()
            path = path.replace("{campaign}", value)
        return path.replace("{account}", "f2" * 16)

    def require_campaign(self):
        if self.campaign_id:
            return self.campaign_id
        client = self.require_member()
        catalog = client.call("GET", "/app/api/campaigns/new")
        self.success(catalog, BY_ID["C19"])
        reply = client.call("POST", "/app/api/campaigns/drafts",
                            {"operation": client.operation()})
        self.success(reply, BY_ID["C19"])
        draft_id = reply.data["draft_id"]
        draft = reply.data["draft"]
        draft.update(name="FLOWMAP synthetic table", adventure_id=catalog.data["adventures"][0]["adventure_id"],
                     content_rating="teen", seat_count=2, hero_choice="in_lobby", has_voice=False,
                     pictures="existing_art_only", pace="story", custom_ceiling_cents_per_hour=None,
                     co_hosts_can_invite=False, co_hosts_can_start=False, co_hosts_can_export=False,
                     join_mode="link_and_room_code", funding="sponsor")
        for step in range(1, 6):
            reply = client.call("PUT", f"/app/api/campaigns/drafts/{draft_id}/steps/{step}",
                                {"draft": draft})
            self.success(reply, BY_ID["C19"])
        operation = client.operation()
        path = f"/app/api/campaigns/drafts/{draft_id}/finish"
        reply = client.call("POST", path, {"operation": operation})
        self.success(reply, BY_ID["C19"])
        replay = client.call("POST", path, {"operation": operation})
        check(replay.status == 200 and replay.data == reply.data, "FLOW-CAMPAIGN",
              "Wizard finish replay must return the same campaign")
        self.campaign_id = reply.data["campaign_id"]
        return self.campaign_id

    def success(self, reply, flow):
        if reply.status in (404, 405):
            raise CheckFailure("MISSING", flow.gap, "Required HTTP route/resource returned 404")
        if reply.status in (501, 503):
            raise CheckFailure("MISSING", flow.gap,
                               f"Real owner boundary unavailable (HTTP {reply.status})")
        check(reply.status == 200, flow.gap, f"Expected 200, observed {reply.status}")
        check(isinstance(reply.data, dict), flow.gap, "Expected typed JSON object")

    def open_page(self, flow, width=1280, locale="en", path=None, origin=None):
        if not importlib.util.find_spec("playwright"):
            self.blocked("Pinned Python Playwright is not installed; no browser observation made",
                         "FLOW-PYTHON")
        if self.browser is None:
            from playwright.sync_api import sync_playwright
            self.playwright = sync_playwright().start()
            try:
                self.browser = self.playwright.chromium.launch(
                    channel=self.args.channel, headless=True,
                    args=["--use-angle=swiftshader", "--enable-unsafe-swiftshader"])
            except Exception as error:
                self.blocked(f"Installed Edge could not launch: {type(error).__name__}", "FLOW-BROWSER")
        self.context = self.browser.new_context(viewport={"width": width, "height": 900},
                                                locale=locale, service_workers="block")
        origins = {self.args.origin, self.args.site_open, self.args.site_closed}

        def local_only(route):
            from urllib.parse import urlsplit
            parsed = urlsplit(route.request.url)
            if f"{parsed.scheme}://{parsed.netloc}" in origins:
                route.continue_()
            else:
                route.abort("blockedbyclient")

        self.context.route("**/*", local_only)
        if flow.id not in STATIC | PUBLIC:
            client = self.require_member()
            # Preserve cookie flags. API fixture injection is not a cookie-delivery/TLS test.
            cookies = [{"name": name, "value": item.value, "domain": "127.0.0.1", "path": "/",
                        "secure": bool(item["secure"]), "httpOnly": bool(item["httponly"]),
                        "sameSite": item["samesite"] or "Lax"}
                       for name, item in client.cookies.items()]
            self.context.add_cookies(cookies)
        self.page = self.context.new_page()
        self.page.set_default_timeout(6000)
        self.page.on("pageerror", lambda error: self.browser_errors.append(type(error).__name__))
        route = path or flow.route
        if "{" in route:
            if "{campaign}" in route:
                route = route.replace("{campaign}", self.require_campaign())
            if "{account}" in route:
                self.blocked("Operator fixture needs an actual separately authorized target account")
            if "{" in route:
                self.blocked("No admitted runtime/share fixture for this route")
        self.page.goto((origin or self.args.origin) + route, wait_until="networkidle")
        return self.page

    def capture(self, case, result):
        if not self.page or result == "PASS":
            return None
        path = self.screens / f"{case.flow}-{case.dimension}-{case.variant}.png"
        try:
            # Token forms must scrub the address before capture; never persist URL/console dumps.
            self.page.screenshot(path=str(path), full_page=True)
            return str(path)
        except Exception as error:
            self.capture_error = f"Screenshot unavailable: {type(error).__name__}"
            return None

    def execute(self, case):
        flow = BY_ID[case.flow]
        dimension = case.dimension
        if dimension == "happy":
            if case.variant == "boundary":
                return self.boundary(flow)
            return self.journey(flow)
        if dimension == "security":
            return self.security(flow, case.variant)
        if dimension == "permissions":
            return self.permissions(flow, case.variant)
        if dimension == "validation":
            return self.validation(flow)
        if dimension in ("viewport", "languages", "accessibility"):
            return self.presentation(flow, dimension, case.variant)
        if dimension in ("concurrency", "idempotency"):
            return self.replay(flow, dimension, case.variant)
        if dimension == "network":
            return self.network(flow, case.variant)
        if dimension == "states":
            return self.states(flow, case.variant)
        if dimension == "provider":
            self.blocked("Native processor/OIDC/runner fake endpoints are not configurable in this hosted binary; "
                         f"cannot drive {case.variant} through the real composition", flow.gap)
        if dimension == "time":
            self.blocked(f"Hosted binary uses SystemClock; no scoped injected-clock control for {case.variant}")
        if dimension == "audit":
            self.blocked("No correlated read-only protected audit/telemetry query fixture supplied; HTTP success is insufficient",
                         "FLOW-AUDIT")

    def boundary(self, flow):
        if flow.area == "admin":
            self.blocked("No fresh-MFA operator fixture; ordinary member denial cannot prove this read", "ADMINFLOW")
        if not flow.endpoint or flow.id in POST:
            self.blocked("This flow needs a workflow observation; no read projection proves it", "FLOW-COVERAGE")
        client = self.client_for(flow)
        reply = client.call("GET", self.endpoint(flow))
        self.success(reply, flow)
        if flow.id == "C17":
            text = json.dumps(reply.data)
            check("10000000" in text, "TENANTISSUE", "First verified account must expose $10 trial")
        self.observation = f"Read-only HTTP projection returned 200; full journey is a separate case ({flow.id})"

    def journey(self, flow):
        if flow.id == "C01":
            return self.site_journey(flow)
        if flow.id in ("C18", "C36", "C40"):
            page = self.open_page(flow, origin=self.args.site_closed if flow.id == "C40" else self.args.site_open)
            check(page.get_by_role("heading", level=1).count() == 1, flow.gap, "One meaningful main heading required")
            if flow.id == "C36":
                check(page.get_by_role("link", name=re.compile("support@", re.I)).count() > 0,
                      flow.gap, "Help needs an accessible support contact")
            if flow.id == "C40":
                check(page.get_by_role("heading", name="Sign-up is coming soon").is_visible(),
                      flow.gap, "Closed deep-link must show coming-soon state")
            self.observation = "Observed public page contract; external contact delivery is not verified"
            return
        if flow.id in ("C02", "C03", "C04", "C05"):
            return self.auth_journey(flow)
        if flow.id == "C19":
            self.require_campaign()
            page = self.open_page(flow, path="/app/dashboard")
            check(page.get_by_text("FLOWMAP synthetic table", exact=True).is_visible(), flow.gap,
                  "Real API-created campaign must be reachable in mounted dashboard")
            self.observation = "Five-step real-API draft/finish/replay and mounted dashboard campaign observed"
            return
        # Probe the mounted feature first: a returned app shell alone never passes the journey.
        if flow.fixture in ("runtime", "session", "share", "bootstrap", "operator", "oauth", "payment", "token"):
            if flow.fixture in ("payment", "oauth", "token"):
                if flow.endpoint and flow.id not in POST:
                    reply = self.client_for(flow).call("GET", self.endpoint(flow))
                    if reply.status in (404, 501, 503):
                        raise CheckFailure("MISSING", flow.gap,
                                           f"Required mounted boundary returned {reply.status}")
            self.blocked("Full journey requires independently provisioned role/provider/runtime/consent fixture", flow.gap)
        self.open_page(flow)
        self.blocked("Mounted view inspected; full action/settlement journey is not yet implemented in this test",
                     "FLOW-COVERAGE")

    def auth_journey(self, flow):
        if flow.id in ("C02", "C03"):
            visitor = Client(self.args.origin)
            visitor.session()
            body = new_signup(visitor)
            if flow.id == "C02":
                page = self.open_page(flow)
                page.get_by_label("Email address", exact=True).fill(body["email"])
                page.get_by_label("Password", exact=True).fill(PASSWORD)
                page.get_by_role("checkbox", name="I am 18 or older", exact=True).check()
                page.get_by_role("checkbox", name="I accept the terms of service and the privacy notice", exact=True).check()
                page.get_by_role("button", name="Create account", exact=True).click()
                page.get_by_role("heading", name="Check your email", exact=True).wait_for()
                mail_token(self.args.outbox, body["email"], "verify-email")
                self.observation = "Browser signup produced verification view and matching fake outbox mail"
                return
            reply = visitor.call("POST", "/app/api/sign-up", body)
            self.success(reply, flow)
            token = mail_token(self.args.outbox, body["email"], "verify-email")
            page = self.open_page(flow, path="/app/verify-email#token=" + token)
            page.get_by_label("New password", exact=True).fill(PASSWORD)
            page.get_by_label("New password again", exact=True).fill(PASSWORD)
            page.get_by_role("button", name="Verify my email", exact=True).click()
            page.wait_for_url("**/app/dashboard")
            verified = page.evaluate("async () => (await fetch('/app/api/session', {credentials:'same-origin'})).json()")
            check(verified.get("account", {}).get("is_verified"), "FLOW-AUTH",
                  "Verification must establish a fresh verified browser session")
            check("#" not in page.url and "token=" not in page.url, "FLOW-AUTH", "Proof must leave address/history")
            self.observation = "Explicit browser verification and token-free address observed"
            return
        client = self.require_member()
        page = self.open_page(flow)
        if flow.id == "C04":
            page.get_by_label("Email address", exact=True).fill(self.email)
            page.get_by_label("Password", exact=True).fill(PASSWORD)
            page.get_by_role("button", name="Sign in", exact=True).click()
            page.wait_for_url("**/app/dashboard")
            page.get_by_role("button", name="Sign out", exact=True).click()
            page.get_by_role("heading", name="You are signed out", exact=True).wait_for()
            self.observation = "Browser signin/dashboard/signout observed"
            return
        page.get_by_label("Email address", exact=True).fill(self.email)
        page.get_by_role("button", name="Email me a reset link", exact=True).click()
        page.get_by_role("heading", name="Check your email", exact=True).wait_for()
        token = mail_token(self.args.outbox, self.email, "reset-password")
        page.goto(self.args.origin + "/app/reset-password#token=" + token)
        page.get_by_label("New password", exact=True).fill(PASSWORD + " again")
        page.get_by_label("New password again", exact=True).fill(PASSWORD + " again")
        page.get_by_role("button", name="Save new password", exact=True).click()
        page.wait_for_url("**/app/sign-in")
        page.get_by_text("Your password has been changed", exact=False).wait_for()
        check(client.call("GET", "/app/api/overview").status == 401, "FLOW-AUTH",
              "Reset must revoke the old fixture session")
        self.member = None
        self.observation = "Browser reset completed and old session refused"

    def site_journey(self, flow):
        for mode, origin in (("open", self.args.site_open), ("closed", self.args.site_closed)):
            for language in ("en", "es", "de", "fr"):
                prefix = "" if language == "en" else "/" + language
                for path in ("/", "/how-it-works/", "/pricing/", "/rules/", "/help/", "/news/",
                             "/changelog/", "/status/", "/legal/terms/", "/legal/privacy/",
                             "/legal/credits/", "/legal/refunds/", "/legal/content/"):
                    self.close_page()
                    page = self.open_page(flow, locale=language, path=prefix + path, origin=origin)
                    check(page.locator("html").get_attribute("lang") == language,
                          "FLOW-LOCALE", f"Incorrect document language at {mode}/{language}/{path}")
                    check(page.get_by_role("heading", level=1).count() == 1,
                          "FLOW-SITE", f"Missing/duplicate heading at {mode}/{language}/{path}")
        self.observation = "104 rendered public pages opened in actual Edge with correct language and heading"

    def validation(self, flow):
        if flow.id not in MISSING_FIELDS:
            if flow.endpoint and flow.id in POST and flow.area != "admin":
                client = self.client_for(flow)
                reply = client.call("POST", self.endpoint(flow, foreign=True), {"unexpected": True})
                if reply.status == 404:
                    raise CheckFailure("MISSING", flow.gap, "Required validation route returned 404")
            self.blocked("Exhaustive domain failure fixtures listed in catalog are not yet available", "FLOW-COVERAGE")
        client = Client(self.args.origin)
        client.session()
        if flow.id == "C02":
            cases = (("email", "", "email_empty"), ("email", "bad", "email_malformed"),
                     ("email", "x" * 255 + "@example.test", "email_too_long"),
                     ("password", "short", "password_too_short"),
                     ("is_adult", False, "adult_required"),
                     ("accepts_terms", False, "terms_required"),
                     ("terms_version", "obsolete", "terms_required"))
            for field, value, refusal in cases:
                body = new_signup(client)
                body[field] = value
                reply = client.call("POST", flow.endpoint, body)
                check(reply.status == 422 and isinstance(reply.data, dict) and reply.data.get("refusal") == refusal,
                      "FLOW-AUTH", f"{field}/{refusal}: expected typed 422, observed {reply.status}")
            self.observation = "Seven named signup validation refusals observed; remaining domain cases are in coverage residuals"
            return
        if flow.id == "C04":
            reply = client.call("POST", flow.endpoint,
                                {"email": "flowmap-absent@example.test", "password": PASSWORD})
            check(reply.status == 422 and reply.data.get("refusal") == "wrong_password",
                  "FLOW-AUTH", "Unknown credentials require generic wrong-password refusal")
        elif flow.id == "C05":
            reply = client.call("POST", flow.endpoint, {"email": "flowmap-absent@example.test"})
            check(reply.status == 200 and reply.data.get("accepted") is True, "FLOW-AUTH",
                  "Unknown reset address must receive accepted response")
        else:
            reply = client.call("POST", flow.endpoint,
                                {"token": "f" * 64, "password": PASSWORD, "confirmation": PASSWORD})
            check(reply.status == 200 and reply.data == "invalid_link", "FLOW-AUTH",
                  "Unknown verification proof must report invalid link")
        self.observation = "Specific negative HTTP case observed; not exhaustive validation approval"

    def security(self, flow, variant):
        if not flow.endpoint:
            self.blocked("No API security boundary for this flow; public/browser security needs separate observation",
                         "FLOW-COVERAGE")
        client = self.client_for(flow, anonymous=flow.id in PUBLIC)
        path = self.endpoint(flow, foreign=True)
        if variant == "headers":
            reply = client.call("GET", path)
            check(reply.headers.get("Cache-Control") == "no-store", flow.gap, "API must be no-store")
            check(reply.headers.get("X-Content-Type-Options") == "nosniff", flow.gap, "API must be nosniff")
            check(reply.headers.get("Referrer-Policy") == "no-referrer", flow.gap, "API must suppress referrers")
            self.observation = f"Security headers observed on GET boundary (HTTP {reply.status}); not authorization proof"
            return
        if variant == "foreign-id":
            if "{campaign}" not in flow.endpoint:
                self.blocked("Need real foreign resource and matching missing-resource fixture")
            own_id = self.require_campaign()
            stranger, _email = create_member(self.args.origin, self.args.outbox)
            foreign = stranger.call("GET", flow.endpoint.replace("{campaign}", own_id))
            missing = stranger.call("GET", path)
            check(foreign.status == missing.status == 404 and foreign.data == missing.data,
                  "FLOW-CAMPAIGN", "Foreign and missing resources must be indistinguishable 404s")
            self.observation = "Separate non-member received identical missing/foreign-campaign 404"
            return
        changes = {"csrf": {"X-DF-CSRF": None}, "origin": {"Origin": "https://attacker.invalid"},
                   "fetch-site": {"Sec-Fetch-Site": "cross-site"}}
        if flow.id == "C06":
            from urllib.parse import urlencode
            headers = dict(changes[variant], **{"Content-Type": "application/x-www-form-urlencoded"})
            body = urlencode({"intent": "sign_in", "csrf": "" if variant == "csrf" else client.csrf}).encode()
            reply = client.call("POST", path, body, headers=headers)
            check(reply.status == 303 and reply.headers.get("Location") == "/app/oauth/error/expired",
                  "GOOGLEAUTH", "OAuth form guard must redirect to the fixed expired refusal")
            self.observation = "OAuth form CSRF/origin refusal redirected only to fixed local expired page"
            return
        method = "POST" if flow.id in POST else "GET"
        if method == "GET":
            self.blocked("Read-only endpoint: mutation CSRF requires the flow's actual mutation route", "FLOW-COVERAGE")
        reply = client.call(method, path, {}, headers=changes[variant])
        if reply.status in (404, 405):
            raise CheckFailure("MISSING", flow.gap, "Required mutation route not mounted")
        check(reply.status == 403 and reply.data.get("code") == "csrf_refused",
              "FLOW-SECURITY", f"{variant} must refuse as csrf_refused, observed {reply.status}")
        self.observation = f"Real mutation guard refused {variant}; no external request made"

    def permissions(self, flow, variant):
        if not flow.endpoint:
            self.blocked("Role boundary needs mounted API/runtime fixture")
        if variant not in ("anonymous", "member"):
            self.blocked(f"Separate {variant} principal/proof fixture not available; no role equivalence assumed")
        if flow.id in POST or flow.id in PUBLIC:
            self.blocked("This role case needs action-specific valid payload and pre/post state", "FLOW-COVERAGE")
        client = self.client_for(flow, anonymous=variant == "anonymous")
        reply = client.call("GET", self.endpoint(flow, foreign=True))
        if flow.area == "admin":
            check(reply.status == 404, "ADMINFLOW",
                  f"D34 concealed denial for {variant}: expected 404, observed {reply.status}")
            self.observation = "Non-operator server-side denial is 404-shaped; no operator success inferred"
        elif variant == "anonymous":
            if flow.id == "C39" and reply.status == 404:
                raise CheckFailure("MISSING", "APIBUILD2", "Integration API not mounted; authentication boundary cannot run")
            check(reply.status == 401, flow.gap, f"Anonymous private read must be 401, observed {reply.status}")
            self.observation = "Anonymous caller refused by private HTTP boundary"
        else:
            self.success(reply, flow)
            self.observation = "Own ordinary member read projection observed; host/non-member cases remain separate"

    def presentation(self, flow, dimension, variant):
        if dimension == "accessibility" and variant == "screen-reader":
            self.blocked("No manual screen-reader observation available; accessible-name checks cannot substitute",
                         "FLOW-A11Y")
        origin = self.args.site_open if flow.id in STATIC else self.args.origin
        prefix = "" if variant == "en" else "/" + variant
        path = prefix + flow.route if dimension == "languages" and flow.id in STATIC else None
        page = self.open_page(flow, width=int(variant) if dimension == "viewport" else 1280,
                              locale=variant if dimension == "languages" else "en", path=path, origin=origin)
        check(page.get_by_role("heading", level=1).count() == 1, flow.gap, "Mounted page needs one main heading")
        if dimension == "viewport":
            overflow = page.evaluate("document.documentElement.scrollWidth > innerWidth + 1")
            check(not overflow, "FLOW-RESPONSIVE", f"Horizontal overflow at {variant}px")
            self.observation = f"Actual rendered page has no horizontal overflow at {variant}px; visual review still required"
        elif dimension == "languages":
            check(page.locator("html").get_attribute("lang") == variant, "FLOW-LOCALE",
                  f"Selected {variant} document language not rendered")
            self.observation = f"Document language {variant} observed; translation quality needs language review"
        else:
            for role in ("button", "textbox", "checkbox", "combobox"):
                for locator in page.get_by_role(role).all():
                    # aria_snapshot uses the browser accessibility tree, including associated labels.
                    snapshot = locator.aria_snapshot()
                    check('"' in snapshot, "FLOW-A11Y", f"Unnamed {role} in accessibility tree")
            for _ in range(4):
                page.keyboard.press("Tab")
            check(page.evaluate("document.activeElement !== document.body"), "FLOW-A11Y",
                  "Keyboard focus must reach a control")
            self.observation = "Four Tab steps and browser accessibility-tree names observed; full keyboard journey/manual AT unverified"

    def replay(self, flow, dimension, variant):
        if flow.id == "C19":
            self.require_campaign()
            self.observation = "Exact finish key replay returned same campaign; concurrent different-key race unperformed"
            if dimension == "concurrency":
                self.blocked("Wizard sequential replay passed; true simultaneous finish requires a barrier fixture", "FLOW-COVERAGE")
            return
        if flow.id != "C02":
            self.blocked("Need valid action payload, operation lookup and durable before/after fixture", "FLOW-COVERAGE")
        client = Client(self.args.origin)
        client.session()
        body = new_signup(client)
        if dimension == "concurrency":
            second = Client(self.args.origin)
            second.cookies = client.cookies.copy()
            second.csrf = client.csrf
            with ThreadPoolExecutor(max_workers=2) as executor:
                replies = list(executor.map(lambda c: c.call("POST", flow.endpoint, body), (client, second)))
        else:
            replies = [client.call("POST", flow.endpoint, body), client.call("POST", flow.endpoint, body)]
        check(all(reply.status == 200 and reply.data == {"accepted": True} for reply in replies),
              "FLOW-AUTH", "Same-key signup replay must have identical enumeration-safe receipt")
        changed = dict(body, password=PASSWORD + " changed")
        conflict = client.call("POST", flow.endpoint, changed)
        check(conflict.status == 409, "FLOW-SIGNUP-REPLAY", "Changed signup payload under same key must conflict")
        self.observation = "Two HTTP clients/same-key signup received accepted once-shaped receipts; changed payload 409; database count not asserted"
        if variant == "two-tabs":
            self.blocked("HTTP concurrent requests observed; two actual browser tabs still need browser execution", "FLOW-COVERAGE")

    def network(self, flow, variant):
        if variant == "lost-ack":
            self.blocked("Need server-side commit barrier and operation-lookup fixture; aborting a request cannot prove commit position")
        page = self.open_page(flow, origin=self.args.site_open if flow.id in STATIC else None)
        if variant == "asset-failure":
            self.context.route("**/*.wasm", lambda route: route.abort("failed"))
            self.context.route("**/*.webp", lambda route: route.abort("failed"))
            page.reload(wait_until="domcontentloaded")
            check(page.get_by_role("heading", level=1).is_visible(), flow.gap,
                  "Asset failure needs a readable fallback heading")
            self.observation = "Image/WASM request failure retained readable heading; audio/runtime recovery unverified"
        else:
            self.context.set_offline(True)
            self.context.set_offline(False)
            page.reload(wait_until="networkidle")
            check(page.get_by_role("heading", level=1).count() == 1, flow.gap,
                  "Reconnect must recover one mounted view")
            self.blocked("Offline toggle/reload observed; mid-flight mutation and reconnect authority not yet asserted",
                         "FLOW-COVERAGE")

    def states(self, flow, variant):
        if flow.area == "admin":
            self.blocked("Empty/loading/error states require a fresh-MFA operator fixture", "ADMINFLOW")
        if not flow.endpoint or flow.id in POST:
            self.blocked("Need per-flow empty/loading/failure projection fixture", "FLOW-COVERAGE")
        if variant == "empty":
            reply = self.client_for(flow).call("GET", self.endpoint(flow))
            self.success(reply, flow)
            self.blocked("Fresh-account projection obtained; mounted empty-state actions are not yet asserted", "FLOW-COVERAGE")
        page = self.open_page(flow)
        target = self.args.origin + self.endpoint(flow)
        if variant == "unavailable":
            self.context.route(target, lambda route: route.fulfill(status=503,
                content_type="application/json", body='{"error":{"code":"unavailable"}}'))
            page.reload(wait_until="networkidle")
            check(page.get_by_role("alert").count() + page.get_by_role("button", name="Try again").count() > 0,
                  flow.gap, "Unavailable projection must offer an announced recovery")
            self.observation = "Explicit injected browser 503 produced announced recovery; this is not a real backend outage"
        else:
            self.blocked("Controlled response-release synchronization needed for deterministic loading observation", "FLOW-COVERAGE")

//! The canonical test vector published with the Standard Webhooks spec,
//! verifying this crate's signature bytes match the reference implementations.
use seiva_webhooks::Webhook;

#[test]
fn matches_the_official_standard_webhooks_vector() {
    let wh = Webhook::new("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();
    let sig = wh.sign(
        "msg_p5jXN8AQM9LWM0D4loKWxJek",
        1_614_265_330,
        b"{\"test\": 2432232314}",
    );
    assert_eq!(
        sig.as_str(),
        "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE="
    );
}

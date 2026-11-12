use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;

use crate::core::{
    clock::{Clock, ManualClock},
    cookie::{
        COOKIE_LIFETIME, CookieError, CookieKey, CookieScope, clear_cookie_header, mint,
        set_cookie_header, verify,
    },
    ids::UserId,
};

fn key() -> CookieKey {
    CookieKey::try_from([7u8; 32].as_slice()).unwrap()
}

fn clock() -> ManualClock {
    ManualClock::new(Timestamp::from_second(1_800_000_000).unwrap())
}

fn session(seed: &str) -> CookieScope {
    CookieScope::Session(seed.parse().unwrap())
}

const A: &str = "aaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbb";

#[test]
fn round_trip_and_scope_binding() {
    let clock = clock();
    let door = mint(&key(), CookieScope::Door, UserId(42), &clock).unwrap();
    let on_a = mint(&key(), session(A), UserId(42), &clock).unwrap();
    assert_eq!(door.len(), 88);

    let verified = verify(&key(), CookieScope::Door, &door, &clock).unwrap();
    assert_eq!(verified.user, UserId(42));
    assert_eq!(
        verified.expires,
        clock.now().checked_add(COOKIE_LIFETIME).unwrap()
    );
    assert_eq!(
        verify(&key(), session(A), &on_a, &clock).unwrap().user,
        UserId(42)
    );

    assert_eq!(
        verify(&key(), session(B), &on_a, &clock),
        Err(CookieError::WrongScope)
    );
    assert_eq!(
        verify(&key(), CookieScope::Door, &on_a, &clock),
        Err(CookieError::WrongScope)
    );
    assert_eq!(
        verify(&key(), session(A), &door, &clock),
        Err(CookieError::WrongScope)
    );
}

#[test]
fn expiry_and_key() {
    let clock = clock();
    let value = mint(&key(), CookieScope::Door, UserId(1), &clock).unwrap();
    clock.advance(COOKIE_LIFETIME - SignedDuration::from_secs(1));
    assert!(verify(&key(), CookieScope::Door, &value, &clock).is_ok());
    clock.advance(SignedDuration::from_secs(1));
    assert_eq!(
        verify(&key(), CookieScope::Door, &value, &clock),
        Err(CookieError::Expired)
    );

    let other = CookieKey::try_from([8u8; 32].as_slice()).unwrap();
    let clock = self::clock();
    assert_eq!(
        verify(&other, CookieScope::Door, &value, &clock),
        Err(CookieError::BadSignature)
    );
    assert_eq!(
        CookieKey::try_from([1u8; 31].as_slice()).unwrap_err(),
        CookieError::ShortKey
    );
}

#[test]
fn malformed_values() {
    let clock = clock();
    for bad in [
        "",
        "abc",
        "====",
        &"A".repeat(87),
        &"A".repeat(89),
        &"A".repeat(88),
    ] {
        assert!(
            verify(&key(), CookieScope::Door, bad, &clock).is_err(),
            "{bad:?}"
        );
    }
    let value = mint(&key(), CookieScope::Door, UserId(1), &clock).unwrap();
    assert!(verify(&key(), CookieScope::Door, &format!("{value}="), &clock).is_err());
    assert!(
        verify(
            &key(),
            CookieScope::Door,
            &format!("+{}", value.get(1..).unwrap()),
            &clock
        )
        .is_err()
    );
}

#[test]
fn header_strings() {
    assert_eq!(
        set_cookie_header(CookieScope::Door, "v"),
        "__Host-jamjam-door=v; Max-Age=604800; Path=/; Secure; HttpOnly; SameSite=Lax"
    );
    assert_eq!(
        set_cookie_header(session(A), "v"),
        "__Host-jamjam=v; Max-Age=604800; Path=/; Secure; HttpOnly; SameSite=Lax"
    );
    assert_eq!(
        clear_cookie_header(CookieScope::Door),
        "__Host-jamjam-door=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Lax"
    );
}

proptest! {
    #[test]
    fn round_trip(user in any::<u64>(), seed in "[a-z2-7]{16}", door in any::<bool>()) {
        let clock = clock();
        let scope = if door { CookieScope::Door } else { session(&seed) };
        let value = mint(&key(), scope, UserId(user), &clock).unwrap();
        let verified = verify(&key(), scope, &value, &clock).unwrap();
        prop_assert_eq!(verified.user, UserId(user));
        prop_assert_eq!(verified.scope, scope);
    }

    #[test]
    fn any_single_character_tamper_fails(
        user in any::<u64>(),
        position in 0usize..88,
        replacement in "[A-Za-z0-9_\\-]",
    ) {
        let clock = clock();
        let value = mint(&key(), session(A), UserId(user), &clock).unwrap();
        let tampered: String = value
            .chars()
            .enumerate()
            .map(|(index, c)| if index == position { replacement.chars().next().unwrap() } else { c })
            .collect();
        prop_assume!(tampered != value);
        prop_assert!(verify(&key(), session(A), &tampered, &clock).is_err());
    }

    #[test]
    fn any_single_byte_tamper_fails(position in 0usize..66, flip in 1u8..=255) {
        let clock = clock();
        let value = mint(&key(), session(A), UserId(5), &clock).unwrap();
        let mut bytes = data_encoding::BASE64URL_NOPAD.decode(value.as_bytes()).unwrap();
        if let Some(byte) = bytes.get_mut(position) {
            *byte ^= flip;
        }
        let tampered = data_encoding::BASE64URL_NOPAD.encode(&bytes);
        prop_assert!(verify(&key(), session(A), &tampered, &clock).is_err());
    }
}

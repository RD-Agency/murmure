from murmure_server.ratelimit import RateLimiter


def test_seau_a_jetons():
    now = [0.0]
    rl = RateLimiter(per_minute=60, clock=lambda: now[0])  # 1 jeton/s, capacité 60
    for _ in range(60):
        assert rl.check("a") is None
    delay = rl.check("a")
    assert delay is not None and 0 < delay <= 1
    assert rl.check("b") is None  # seaux indépendants par clé
    now[0] += 1.0
    assert rl.check("a") is None


def test_desactive():
    rl = RateLimiter(per_minute=0)
    assert not rl.enabled and all(rl.check("a") is None for _ in range(1000))

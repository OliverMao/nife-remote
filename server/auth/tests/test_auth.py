"""账号服务的测试：注册、登录、退出、凭证、设备列表、错误码、文件落盘。

跑法（在 server/auth 目录）：

    python -m pip install -r requirements-dev.txt
    cd server/auth && python -m pytest -q

每个测试用一个临时目录里的 accounts.json，不碰真实数据。
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app import settings as settings_mod  # noqa: E402
from app import store as store_mod  # noqa: E402
from app.store import AccountStore  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    """每个测试一份新的账号文件（迭代次数调小，测试才快）"""
    db = tmp_path / "accounts.json"
    monkeypatch.setenv("NIFE_REMOTE_AUTH_DB", str(db))
    monkeypatch.setenv("NIFE_REMOTE_AUTH_ROUNDS", "1000")
    monkeypatch.setenv("NIFE_REMOTE_AUTH_DATA", str(tmp_path))
    from app import main as main_mod  # 同一个模块：StoreError 等类型只有一份

    main_mod.settings = settings_mod.Settings(
        db_path=db,
        token_ttl=3600,
        max_users=5,
        registration="open",
        rounds=1000,
        relay_host="relay.test",
        relay_port=4433,
        relay_fingerprint="ab:cd",
    )
    main_mod.store = main_mod.AccountStore(db, rounds=1000, token_ttl=3600, max_users=5)
    main_mod._failures.clear()
    return TestClient(main_mod.app), db


def test_health(client):
    c, _ = client
    r = c.get("/health")
    assert r.status_code == 200
    assert r.json()["ok"] is True
    assert r.json()["users"] == 0


def test_register_returns_token_and_user(client):
    c, db = client
    r = c.post("/auth/register", json={"name": "mao", "password": "hunter2"})
    assert r.status_code == 200
    body = r.json()
    assert body["user"]["name"] == "mao"
    assert len(body["token"]) >= 32
    assert "hash" not in body["user"] and "salt" not in body["user"]
    # 落盘了，而且文件里没有明文密码
    saved = db.read_text(encoding="utf-8")
    assert "mao" in saved
    assert "hunter2" not in saved
    data = json.loads(saved)
    assert data["users"][0]["rounds"] == 1000


def test_duplicate_name_is_rejected(client):
    c, _ = client
    assert c.post("/auth/register", json={"name": "mao", "password": "hunter2"}).status_code == 200
    r = c.post("/auth/register", json={"name": "MAO", "password": "hunter2"})
    assert r.status_code == 400
    assert r.json()["error"]["code"] == "name_taken"


def test_short_password_and_bad_name(client):
    c, _ = client
    r = c.post("/auth/register", json={"name": "mao", "password": "123"})
    assert r.status_code == 400
    assert r.json()["error"]["code"] == "bad_password"
    r = c.post("/auth/register", json={"name": "m", "password": "hunter2"})
    assert r.status_code == 400
    assert r.json()["error"]["code"] == "bad_name"


def test_login_and_me(client):
    c, _ = client
    c.post("/auth/register", json={"name": "mao", "password": "hunter2"})
    r = c.post("/auth/login", json={"name": "mao", "password": "hunter2"})
    assert r.status_code == 200
    token = r.json()["token"]
    r = c.get("/auth/me", headers={"Authorization": f"Bearer {token}"})
    assert r.status_code == 200
    assert r.json()["user"]["name"] == "mao"


def test_wrong_password(client):
    c, _ = client
    c.post("/auth/register", json={"name": "mao", "password": "hunter2"})
    r = c.post("/auth/login", json={"name": "mao", "password": "nope-nope"})
    assert r.status_code == 401
    assert r.json()["error"]["code"] == "wrong_password"


def test_unknown_user(client):
    c, _ = client
    r = c.post("/auth/login", json={"name": "nobody", "password": "hunter2"})
    assert r.status_code == 404
    assert r.json()["error"]["code"] == "no_such_user"


def test_me_without_token(client):
    c, _ = client
    assert c.get("/auth/me").status_code == 401


def test_logout_invalidates_the_token(client):
    c, _ = client
    token = c.post("/auth/register", json={"name": "mao", "password": "hunter2"}).json()["token"]
    assert c.post("/auth/logout", headers={"Authorization": f"Bearer {token}"}).status_code == 200
    r = c.get("/auth/me", headers={"Authorization": f"Bearer {token}"})
    assert r.status_code == 401
    assert r.json()["error"]["code"] == "bad_token"


def test_devices_roundtrip_and_validation(client):
    c, _ = client
    token = c.post("/auth/register", json={"name": "mao", "password": "hunter2"}).json()["token"]
    headers = {"Authorization": f"Bearer {token}"}
    assert c.get("/auth/devices", headers=headers).json()["devices"] == []
    r = c.put("/auth/devices", headers=headers, json={"devices": ["260 778", "012345", "12345", "260778"]})
    assert r.status_code == 200
    # 只留合法的 6 位、去重
    assert r.json()["devices"] == ["260778"]
    assert c.get("/auth/devices", headers=headers).json()["devices"] == ["260778"]


def test_registration_can_be_closed(client):
    c, _ = client
    from app import main as main_mod

    main_mod.settings = settings_mod.Settings(db_path=Path("/tmp/x.json"), token_ttl=60, max_users=5, registration="closed", rounds=1000)
    r = c.post("/auth/register", json={"name": "mao", "password": "hunter2"})
    assert r.status_code == 403
    assert r.json()["error"]["code"] == "closed"


def test_store_roundtrip_and_broken_file(tmp_path):
    path = tmp_path / "accounts.json"
    s = store_mod.AccountStore(path, rounds=1000, token_ttl=60, max_users=5)
    user, token = s.register("mao", "hunter2")
    assert s.user_for_token(token.token).id == user.id
    # 重新加载后还在
    again = store_mod.AccountStore(path, rounds=1000, token_ttl=60, max_users=5)
    assert again.login("MAO", "hunter2")[0].id == user.id
    # 坏文件当作没有账号
    path.write_text("{ not json", encoding="utf-8")
    broken = store_mod.AccountStore(path, rounds=1000, token_ttl=60, max_users=5)
    assert broken.user_count == 0


def test_expired_token_is_rejected(tmp_path, monkeypatch):
    path = tmp_path / "accounts.json"
    s = store_mod.AccountStore(path, rounds=1000, token_ttl=-1, max_users=5)
    _user, token = s.register("mao", "hunter2")
    with pytest.raises(store_mod.StoreError) as e:
        s.user_for_token(token.token)
    assert e.value.code == "bad_token"


def test_change_password(client):
    c, _ = client
    token = c.post("/auth/register", json={"name": "mao", "password": "hunter2"}).json()["token"]
    headers = {"Authorization": f"Bearer {token}"}
    # 旧密码不对
    r = c.post("/auth/password", headers=headers, json={"old_password": "nope-nope", "new_password": "hunter3"})
    assert r.status_code == 401
    assert r.json()["error"]["code"] == "wrong_password"
    # 改成功：返回新凭证，旧凭证失效
    r = c.post("/auth/password", headers=headers, json={"old_password": "hunter2", "new_password": "hunter3"})
    assert r.status_code == 200
    fresh = r.json()["token"]
    assert c.get("/auth/me", headers=headers).status_code == 401
    assert c.get("/auth/me", headers={"Authorization": f"Bearer {fresh}"}).status_code == 200
    # 新密码能登录，旧密码不能
    assert c.post("/auth/login", json={"name": "mao", "password": "hunter3"}).status_code == 200
    assert c.post("/auth/login", json={"name": "mao", "password": "hunter2"}).status_code == 401


def test_default_account_is_seeded(client):
    c, _ = client
    from app import main as main_mod

    main_mod.seed_default_account()
    # 再调一次不会重复建
    main_mod.seed_default_account()
    assert main_mod.store.user_count == 1
    r = c.post("/auth/login", json={"name": "niferemote", "password": "niferemote"})
    assert r.status_code == 200
    assert r.json()["user"]["name"] == "niferemote"
    assert r.json()["user"]["devices"] == []


def test_relay_is_assigned_by_the_service(client):
    c, _ = client
    r = c.get("/relay")
    assert r.status_code == 200
    relay = r.json()["relay"]
    assert relay == {"host": "relay.test", "port": 4433, "fingerprint": "ab:cd"}
    # 没配中转服务器时是空字符串（客户端据此提示"账号服务还没分配中转服务器"）
    body = c.get("/health").json()
    assert body["relay"] == "relay.test"


def test_relay_without_configuration(client):
    c, _ = client
    from app import main as main_mod

    main_mod.settings = settings_mod.Settings(
        db_path=Path("/tmp/x.json"), token_ttl=60, max_users=5, registration="open", rounds=1000
    )
    assert c.get("/relay").json()["relay"] == {"host": "", "port": 4433, "fingerprint": ""}
    assert c.get("/health").json()["relay"] is None

(() => {
  globalThis.tysel.sqlite = {
    exec(sql, params) {
      return tysel._sqliteExec(String(sql), JSON.stringify(params == null ? [] : params));
    },
    query(sql, params) {
      return tysel._sqliteQuery(String(sql), JSON.stringify(params == null ? [] : params));
    },
  };
  globalThis.tysel.postgres = {
    exec(sql, params) {
      return tysel._pgExec(String(sql), JSON.stringify(params == null ? [] : params));
    },
    query(sql, params) {
      return tysel._pgQuery(String(sql), JSON.stringify(params == null ? [] : params));
    },
  };
  globalThis.tysel.redis = {
    get(key) {
      return tysel._redisGet(String(key));
    },
    set(key, value, options) {
      const ttl = options == null ? undefined : options.ttlSeconds;
      return tysel._redisSet(String(key), String(value), ttl);
    },
    del(...keys) {
      return tysel._redisDel(JSON.stringify(keys.map(String)));
    },
    exists(key) {
      return tysel._redisExists(String(key));
    },
    expire(key, ttlSeconds) {
      return tysel._redisExpire(String(key), ttlSeconds);
    },
  };
  globalThis.tysel.fs = {
    read(path) {
      return tysel._fsRead(String(path));
    },
    write(path, data) {
      return tysel._fsWrite(String(path), data == null ? "" : String(data));
    },
  };
  globalThis.tysel.secrets = {
    ref(name) {
      return tysel._secretRef(String(name));
    },
  };
})();

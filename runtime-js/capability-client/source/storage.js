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

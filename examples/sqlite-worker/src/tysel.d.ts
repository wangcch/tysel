/** Ambient types for the Tysel host object. */
interface TyselSqlite {
  exec(sql: string, params?: unknown[]): Promise<number>;
  query(sql: string, params?: unknown[]): Promise<Record<string, unknown>[]>;
}

interface Tysel {
  sqlite: TyselSqlite;
}

declare const tysel: Tysel;

/** Ambient types for the Tysel host object. */
interface TyselPostgres {
  exec(sql: string, params?: unknown[]): Promise<number>;
  query(sql: string, params?: unknown[]): Promise<Record<string, unknown>[]>;
}

interface Tysel {
  postgres: TyselPostgres;
}

declare const tysel: Tysel;

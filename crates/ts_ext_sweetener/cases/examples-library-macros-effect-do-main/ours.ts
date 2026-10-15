

import { Context, Data, Effect, Layer } from "effect";

export class DbError extends Data.TaggedError("DbError")<{
readonly sql: string;
}> {}
export class NotFound extends Data.TaggedError("NotFound")<{
readonly id: string;
}> {}

export interface DatabaseShape {
        readonly query: (
            sql: string
          ) => Effect.Effect<readonly string[], DbError>;readonly lookup: (
            id: string
          ) => Effect.Effect<string, NotFound>;
      }

    export class Database extends Context.Tag("Database")<
      Database,
      DatabaseShape
      >() {}

    // One accessor per method, so a caller never reaches for the tag itself.
    export const query = (
          sql: string
        ): Effect.Effect<readonly string[], DbError, Database> =>
        Effect.flatMap(Database, (service) =>
          service.query(sql));export const lookup = (
          id: string
        ): Effect.Effect<string, NotFound, Database> =>
        Effect.flatMap(Database, (service) =>
          service.lookup(id));

    export const DatabaseLayer = (
        implementation: DatabaseShape
      ) => Layer.succeed(Database, implementation);

export const report = ((id: string) => Effect.gen(function* () { const user = (yield* lookup(id));
const rows = (yield* query(`posts for ${user}`));
const count = rows.length;
return `${user}: ${count}`;
 }));

export const safe = (id: string) => Effect.catchTags(report(id), { NotFound: ((error) => Effect.succeed(`no such user ${error.id}`)), DbError: ((error) => Effect.succeed(`query failed: ${error.sql}`)) });

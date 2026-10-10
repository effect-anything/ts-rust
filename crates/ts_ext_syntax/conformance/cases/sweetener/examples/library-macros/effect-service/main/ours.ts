
import { Context, Data, Effect, Layer } from "effect";

export class DbError extends Data.TaggedError("DbError")<{
        readonly sql.name: sql.kind;
      }> {}
export class NotFound extends Data.TaggedError("NotFound")<{
        readonly id.name: id.kind;
      }> {}

export interface DatabaseShape {
        readonly : (
            query(sql: string): Effect<readonly string[], DbError>.parameters.name: query(sql: string): Effect<readonly string[], DbError>.parameters.kind
          ) => Effect.Effect<query(sql: string): Effect<readonly string[], DbError>.ok, query(sql: string): Effect<readonly string[], DbError>.fails>;
      }

    export class Database extends Context.Tag("Database")<
      Database,
      DatabaseShape
      >() {}

    // One accessor per method, so a caller never reaches for the tag itself.
    export const query(sql: string): Effect<readonly string[], DbError>.name = (
          query(sql: string): Effect<readonly string[], DbError>.parameters.name: query(sql: string): Effect<readonly string[], DbError>.parameters.kind
        ): Effect.Effect<query(sql: string): Effect<readonly string[], DbError>.ok, query(sql: string): Effect<readonly string[], DbError>.fails, Database> =>
        Effect.flatMap(Database, (service) =>
          service.query(sql: string): Effect<readonly string[], DbError>.name(query(sql: string): Effect<readonly string[], DbError>.parameters.name));

    export const DatabaseLayer = (
        implementation: DatabaseShape
      ) => Layer.succeed(Database, implementation)

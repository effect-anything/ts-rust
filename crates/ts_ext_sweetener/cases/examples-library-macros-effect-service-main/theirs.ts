
import { Context, Data, Effect, Layer } from "effect";

export class DbError extends Data.TaggedError("DbError")<{ readonly sql: string;
      }> {}
export class NotFound extends Data.TaggedError("NotFound")<{ readonly id: string;
      }> {}

export interface DatabaseShape { readonly query: (sql: string
          ) => Effect.Effect<(readonly string[]), DbError>;
          readonly lookup: (id: string
          ) => Effect.Effect<string, NotFound>;
      }

    export class Database extends Context.Tag("Database")<Database,
      DatabaseShape
      >() {} export const query = ((sql: string
        ): Effect.Effect<(readonly string[]), DbError, Database> =>
        Effect.flatMap(Database, (service) =>
          service.query(sql)));
          export const lookup = ((id: string
        ): Effect.Effect<string, NotFound, Database> =>
        Effect.flatMap(Database, (service) =>
          service.lookup(id)));

    export const DatabaseLayer = ((
        implementation: DatabaseShape
      ) => Layer.succeed(Database, implementation));

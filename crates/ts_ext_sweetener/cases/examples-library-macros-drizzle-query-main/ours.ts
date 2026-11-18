
import { and, asc, desc, eq, gt, gte, lt, lte, ne, or } from "drizzle-orm";
import type { NodePgDatabase } from "drizzle-orm/node-postgres";
import { boolean, integer, pgTable, serial, text } from "drizzle-orm/pg-core";

export const users = pgTable("users", {
    id: serial().primaryKey(),
    name: text().notNull(),
    age: integer().notNull(),
  });

export const posts = pgTable("posts", {
    id: serial().primaryKey(),
    authorId: integer().notNull(),
    title: text().notNull(),
    published: boolean().notNull(),
  });

export const feed = (db: NodePgDatabase, minAge: number) =>
db.select({ author: users.name, title: posts.title }).from(users).innerJoin(posts, eq(posts.authorId, users.id)).where(and(gte(users.age, minAge), eq(posts.published, true))).orderBy(asc(users.name), desc(posts.id)).limit(20);

export const everyone = (db: NodePgDatabase) =>
db.select({ id: users.id, name: users.name }).from(users);

export const quiet = (db: NodePgDatabase, name: string) =>
db.select({ name: users.name, title: posts.title }).from(users).leftJoin(posts, and(eq(posts.authorId, users.id), gt(posts.id, 0))).where(and(ne(users.name, name), lt(users.age, 65), lte(users.age, 64))).orderBy(desc(users.age));

export const either = (db: NodePgDatabase, name: string) =>
db.select({ name: users.name }).from(users).where(or(eq(users.name, name), and(gt(users.age, 65), or(lt(users.id, 10), gt(users.id, 100)))));

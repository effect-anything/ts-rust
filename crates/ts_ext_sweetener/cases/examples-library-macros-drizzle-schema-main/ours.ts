
import {
  boolean,
  integer,
  pgTable,
  serial,
  text,
  timestamp,
  varchar,
} from "drizzle-orm/pg-core";

export const users = pgTable("users", {
id: serial().notNull().primaryKey(), email: varchar({ length: 255 }).notNull().unique(), name: text().notNull(), bio: text(), createdAt: timestamp().notNull().defaultNow()
})
export type User = typeof users.$inferSelect;
export type NewUser = typeof users.$inferInsert;

export const posts = pgTable("posts", {
id: serial().notNull().primaryKey(), authorId: integer().notNull(), title: text().notNull(), published: boolean().notNull().default(false)
})
export type Post = typeof posts.$inferSelect;
export type NewPost = typeof posts.$inferInsert;

// Both types are ordinary Drizzle inference, so they track the columns.
export const draft: NewPost = { authorId: 1, title: "Hello" };
export const byline = (user: User): string => user.bio ?? user.name;

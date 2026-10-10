
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
          id.name: id.kind.builder  { id.kind.arguments } #else { () }
             {} #else { .notNull() }
             { .primaryKey() }
              #else {  { .unique() }
                #else {  { .defaultNow() }
                  #else { .default(id.modifiers.value) } } }
        })

    export type User = typeof users.$;
    export type NewUser = typeof users.$

export const posts = pgTable("posts", {
          id.name: id.kind.builder  { id.kind.arguments } #else { () }
             {} #else { .notNull() }
             { .primaryKey() }
              #else {  { .unique() }
                #else {  { .defaultNow() }
                  #else { .default(id.modifiers.value) } } }
        })

    export type Post = typeof posts.$;
    export type NewPost = typeof posts.$

// Both types are ordinary Drizzle inference, so they track the columns.
export const draft: NewPost = { authorId: 1, title: "Hello" };
export const byline = (user: User): string => user.bio ?? user.name;

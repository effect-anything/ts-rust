

route UserPost = users/:userId/posts/:postId;
export interface Health {
         { readonly health.name: string; }
      }

    export const HealthPattern =
      "/" + [ { ":" + "health" }
          #else { "health" }].join("/")

    export const buildHealth = (parameters: Health): string =>
      "/" + [ { parameters["health"] }
          #else { "health" }].join("/")

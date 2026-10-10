

type Option<T> =
      {
        readonly tag: "None()";
        None().fields.name: None().fields.valueType;
      }|{
        readonly tag: "Some(value: T)";
        Some(value: T).fields.name: Some(value: T).fields.valueType;
      };

    const None().name = <T>(
      None().fields.name: None().fields.valueType
    ): Option<T> => ({
      tag: "None()"
       {
        , None().fields.name
      }
    });const Some(value: T).name = <T>(
      Some(value: T).fields.name: Some(value: T).fields.valueType
    ): Option<T> => ({
      tag: "Some(value: T)"
       {
        , Some(value: T).fields.name
      }
    });

export const result = match (Some(3)) {
  Some(value) => value + 1;
  None() => 0;
};

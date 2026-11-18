import {
  useCallback,
  useContext,
  useDeferredValue,
  useEffect,
  useId,
  useMemo,
  useReducer,
  useRef,
  useState,
  useTransition,
} from "react";

export syntax state:stmt {
  rule { state $value:binding = $initial:expr; }
  bind $value in following as lexical value;
  bind #join($value, prefix: "set", casing: "upper-first") in following as lexical value;
  => {
    const [$value, #join($value, prefix: "set", casing: "upper-first")] = useState($initial);
  }

  rule { state $value:binding: $valueType:type = $initial:expr; }
  bind $value in following as lexical value;
  bind #join($value, prefix: "set", casing: "upper-first") in following as lexical value;
  => {
    const [$value, #join($value, prefix: "set", casing: "upper-first")] = useState<$valueType>($initial);
  }
}

export syntax states:stmt {
  rule { states { $($value:binding = $initial:expr;)* } }
  bind $value in following as lexical value;
  bind #join($value, prefix: "set", casing: "upper-first") in following as lexical value;
  => {
    $(const [$value, #join($value, prefix: "set", casing: "upper-first")] = useState($initial);)*
  }
}

export syntax memo:stmt {
  rule { memo $name:binding [$($dependency:expr),*] = $value:expr; }
  bind $name in following as lexical value;
  => { const $name = useMemo(() => $value, [$($dependency),*]); }

  rule {
    memo $name:binding [$($dependency:expr),*] as $valueType:type = $value:expr;
  }
  bind $name in following as lexical value;
  => { const $name: $valueType = useMemo(() => $value, [$($dependency),*]); }
}

export syntax callback:stmt {
  rule {
    callback $name:binding [$($dependency:expr),*]
    $parameters:tt $body:stmt
  }
  bind $name in following as lexical value;
  => { const $name = useCallback($parameters => $body, [$($dependency),*]); }

  rule {
    callback async $name:binding [$($dependency:expr),*]
    $parameters:tt $body:stmt
  }
  bind $name in following as lexical value;
  => {
    const $name = useCallback(async $parameters => $body, [$($dependency),*]);
  }
}

export syntax effect:stmt {
  rule { effect [$($dependency:expr),*] $body:stmt }
  => { useEffect(() => $body, [$($dependency),*]); }

  rule { effect always $body:stmt }
  => { useEffect(() => $body); }

  rule { effect once $body:stmt }
  => { useEffect(() => $body, []); }
}

export syntax reducer:stmt {
  rule {
    reducer $state:binding with $dispatch:binding
    using $reducer:expr initially $initial:expr;
  }
  bind $state in following as lexical value;
  bind $dispatch in following as lexical value;
  => { const [$state, $dispatch] = useReducer($reducer, $initial); }
}

export syntax ref:stmt {
  rule { ref $name:binding = $initial:expr; }
  bind $name in following as lexical value;
  => { const $name = useRef($initial); }

  rule { ref $name:binding: $valueType:type = $initial:expr; }
  bind $name in following as lexical value;
  => { const $name = useRef<$valueType>($initial); }
}

export syntax context:stmt {
  rule { context $name:binding from $source:expr; }
  bind $name in following as lexical value;
  => { const $name = useContext($source); }
}

export syntax toggle:stmt {
  rule { toggle $value:binding = $initial:expr; }
  bind $value in following as lexical value;
  bind #join($value, prefix: "set", casing: "upper-first") in following as lexical value;
  bind #join($value, prefix: "toggle", casing: "upper-first") in following as lexical value;
  => {
    const [$value, #join($value, prefix: "set", casing: "upper-first")] = useState($initial);
    const #join($value, prefix: "toggle", casing: "upper-first") = useCallback(
      () => {
        #join($value, prefix: "set", casing: "upper-first")((current) => !current);
      },
      [],
    );
  }
}

export syntax deferred:stmt {
  rule { deferred $name:binding = $value:expr; }
  bind $name in following as lexical value;
  => { const $name = useDeferredValue($value); }
}

export syntax reactId:stmt {
  rule { reactId $name:binding; }
  bind $name in following as lexical value;
  => { const $name = useId(); }
}

export syntax latestRef:stmt {
  rule { latestRef $name:binding = $value:expr; }
  bind $name in following as lexical value;
  => {
    const $name = useRef($value);
    useEffect(() => {
        $name.current = $value;
      }, [$value]);
  }
}

export syntax transition:stmt {
  rule { transition $name:binding; }
  bind #join($name, suffix: "Pending") in following as lexical value;
  bind #join($name, prefix: "start", casing: "upper-first") in following as lexical value;
  => {
    const [
      #join($name, suffix: "Pending"),
      #join($name, prefix: "start", casing: "upper-first"),
    ] = useTransition();
  }
}

"use sweetener";

export syntax duplicate:expr {
  rule { duplicate($value:tt) } => { [$value, $value] }
}

export syntax unless:stmt {
  rule { unless ($test:expr) $body:stmt } => { if (!($test)) $body }
}

export operator (|>):expr {
  fixity infix;
  associativity left;
  precedence 40;
  rule { $value:expr |> $callee:ident } => { $callee($value) }
}

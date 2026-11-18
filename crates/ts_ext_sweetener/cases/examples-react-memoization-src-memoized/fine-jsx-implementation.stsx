import { c as _c } from "react/compiler-runtime";

export syntax emitCompilerFixture:item {
  rule {
    emitCompilerFixture($name:binding, $propsType:type)
  }
  bind $name in following as recursive value;
  => {
    #core(function $name(input: $propsType) {
        const cache = _c(9);
        const { cond: conditionInput, id } = input;
        const condition = conditionInput === undefined ? false : conditionInput;
        let className;
        if (cache[0] !== id) {
          className = identity(styles.a, id !== null ? styles.b : {});
          cache[0] = id;
          cache[1] = className;
        } else {
          className = cache[1];
        }
        let firstChild;
        if (cache[2] !== className) {
          firstChild = <div className={className} />;
          cache[2] = className;
          cache[3] = firstChild;
        } else {
          firstChild = cache[3];
        }
        let secondChild;
        if (cache[4] !== condition) {
          secondChild = condition === false && (
            <div className={identity(styles.c, DISPLAY ? styles.d : {})} />
          );
          cache[4] = condition;
          cache[5] = secondChild;
        } else {
          secondChild = cache[5];
        }
        let result;
        if (cache[6] !== firstChild || cache[7] !== secondChild) {
          result = (
            <>
            {firstChild}
            {secondChild}
          </>
          );
          cache[6] = firstChild;
          cache[7] = secondChild;
          cache[8] = result;
        } else {
          result = cache[8];
        }
        return result;
      })
  }
}

export syntax coreItem:item {
  rule { coreItem $declaration:tt }
  => { #core($declaration) }
}

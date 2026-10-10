

#syntax {
      syntax between:expr {
        rule {
          between()
          lower()upper()
        }
        expect "upper argument segment";
        => {
          ((
              value: number,
              minimum: lower(minimum: number)
upper(maximum: number), maximum: lower(minimum: number)
upper(maximum: number)
            ): boolean => { return value > minimum && value < maximum; })(
            ,
            
          )
        }
      }
    }

export const result = between(3)
lower(1)
upper(5);

// A generalised method name is a request grammar. Repetition and optional
// groups use the same regular-language notation as Sweetener patterns.
#syntax {
      syntax query:expr {
        
          rule { query($source:expr)
    $(Where($filter:expr))+
    $(OrderBy($ordering:expr) $(ThenBy($tiebreak:expr))*)?
    Select($projection:expr) }
          expect "a query ending in Select(...) or GroupBy(...)";
          => { request {
    query($source:expr)
    $(Where($filter:expr))+
    $(OrderBy($ordering:expr) $(ThenBy($tiebreak:expr))*)?
    Select($projection:expr)
  }
  expect "a query ending in Select(...) or GroupBy(...)";
  => {
    ({
        source: $source,
        filters: [$($filter),*],
        projection: $projection,
      })
  }

  request {
    query($source:expr)
    $(Where($filter:expr))*
    GroupBy($projection:expr)
  }
  expect "a query ending in Select(...) or GroupBy(...)";
  => {
    ({
        source: $source,
        filters: [$($filter),*],
        projection: $projection,
      })
  } }
        
          rule { query($source:expr)
    $(Where($filter:expr))*
    GroupBy($projection:expr) }
          expect "a query ending in Select(...) or GroupBy(...)";
          => { request {
    query($source:expr)
    $(Where($filter:expr))+
    $(OrderBy($ordering:expr) $(ThenBy($tiebreak:expr))*)?
    Select($projection:expr)
  }
  expect "a query ending in Select(...) or GroupBy(...)";
  => {
    ({
        source: $source,
        filters: [$($filter),*],
        projection: $projection,
      })
  }

  request {
    query($source:expr)
    $(Where($filter:expr))*
    GroupBy($projection:expr)
  }
  expect "a query ending in Select(...) or GroupBy(...)";
  => {
    ({
        source: $source,
        filters: [$($filter),*],
        projection: $projection,
      })
  } }
        
      }
    }

const students = [{ name: "Ada", age: 36 }];
const adult = (student: (typeof students)[number]) => student.age >= 18;
const name = (student: (typeof students)[number]) => student.name;

export const selected = query(students)
Where(adult)
OrderBy(name)
Select(name);

export const grouped = query(students)
GroupBy((student: (typeof students)[number]) => student.age);

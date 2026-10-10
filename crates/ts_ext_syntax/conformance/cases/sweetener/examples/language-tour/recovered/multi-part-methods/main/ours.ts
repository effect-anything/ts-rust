



export const result = between(3)
lower(1)
upper(5);

// A generalised method name is a request grammar. Repetition and optional
// groups use the same regular-language notation as Sweetener patterns.


const students = [{ name: "Ada", age: 36 }];
const adult = (student: (typeof students)[number]) => student.age >= 18;
const name = (student: (typeof students)[number]) => student.name;

export const selected = query(students)
Where(adult)
OrderBy(name)
Select(name);

export const grouped = query(students)
GroupBy((student: (typeof students)[number]) => student.age);



export const result = ((value: number, minimum: number, maximum: number
            ): boolean => {
  return (value > minimum && value < maximum);
            })(3, 1, 5
          );

const students = [{ name: "Ada", age: 36 }];
const adult = (student: (typeof students)[number]) => student.age >= 18;
const name = (student: (typeof students)[number]) => student.name;

export const selected = ({
        source: students,
        filters: [adult],
        projection: name,
      });

export const grouped = ({
        source: students,
        filters: [],
        projection: ((student: (typeof students)[number]) => student.age),
      });

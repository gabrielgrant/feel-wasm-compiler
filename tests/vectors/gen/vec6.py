exec(open('tests/vectors/gen/vec5.py').read().replace("open('tests/vectors/semantic.json','w')", "open('/dev/null','w')"))
# Table 78: every example of the range functions, verbatim from DMN 1.5
T78 = """before( 1, 10 ) = true
before( 10, 1 ) = false
before( 1, [1..10] ) = false
before( 1, (1..10] ) = true
before( 1, [5..10] ) = true
before( [1..10], 10 ) = false
before( [1..10), 10 ) = true
before( [1..10], 15 ) = true
before( [1..10], [15..20] ) = true
before( [1..10], [10..20] ) = false
before( [1..10), [10..20] ) = true
before( [1..10], (10..20] ) = true
after( 10, 5 ) = true
after( 5, 10 ) = false
after( 12, [1..10] ) = true
after( 10, [1..10) ) = true
after( 10, [1..10] ) = false
after( [11..20], 12 ) = false
after( [11..20], 10 ) = true
after( (11..20], 11 ) = true
after( [11..20], 11 ) = false
after( [11..20], [1..10] ) = true
after( [1..10], [11..20] ) = false
after( [11..20], [1..11) ) = true
after( (11..20], [1..11] ) = true
meets( [1..5], [5..10] ) = true
meets( [1..5), [5..10] ) = false
meets( [1..5], (5..10] ) = false
meets( [1..5], [6..10] ) = false
met by( [5..10], [1..5] ) = true
met by( [5..10], [1..5) ) = false
met by( (5..10], [1..5] ) = false
met by( [6..10], [1..5] ) = false
overlaps( [1..5], [3..8] ) = true
overlaps( [3..8], [1..5] ) = true
overlaps( [1..8], [3..5] ) = true
overlaps( [3..5], [1..8] ) = true
overlaps( [1..5], [6..8] ) = false
overlaps( [6..8], [1..5] ) = false
overlaps( [1..5], [5..8] ) = true
overlaps( [1..5], (5..8] ) = false
overlaps( [1..5), [5..8] ) = false
overlaps( [1..5), (5..8] ) = false
overlaps( [5..8], [1..5] ) = true
overlaps( (5..8], [1..5] ) = false
overlaps( [5..8], [1..5) ) = false
overlaps( (5..8], [1..5) ) = false
overlaps before( [1..5], [3..8] ) = true
overlaps before( [1..5], [6..8] ) = false
overlaps before( [1..5], [5..8] ) = true
overlaps before( [1..5], (5..8] ) = false
overlaps before( [1..5), [5..8] ) = false
overlaps before( [1..5), (1..5] ) = true
overlaps before( [1..5], (1..5] ) = true
overlaps before( [1..5), [1..5] ) = false
overlaps before( [1..5], [1..5] ) = false
overlaps after( [3..8], [1..5]) = true
overlaps after( [6..8], [1..5]) = false
overlaps after( [5..8], [1..5]) = true
overlaps after( (5..8], [1..5]) = false
overlaps after( [5..8], [1..5)) = false
overlaps after( (1..5], [1..5) ) = true
overlaps after( (1..5], [1..5] ) = true
overlaps after( [1..5], [1..5) ) = false
overlaps after( [1..5], [1..5] ) = false
finishes( 10, [1..10] ) = true
finishes( 10, [1..10) ) = false
finishes( [5..10], [1..10] ) = true
finishes( [5..10), [1..10] ) = false
finishes( [5..10), [1..10) ) = true
finishes( [1..10], [1..10] ) = true
finishes( (1..10], [1..10] ) = true
finished by( [1..10], 10 ) = true
finished by( [1..10), 10 ) = false
finished by( [1..10], [5..10] ) = true
finished by( [1..10], [5..10) ) = false
finished by( [1..10), [5..10) ) = true
finished by( [1..10], [1..10] ) = true
finished by( [1..10], (1..10] ) = true
includes( [1..10], 5 ) = true
includes( [1..10], 12 ) = false
includes( [1..10], 1 ) = true
includes( [1..10], 10 ) = true
includes( (1..10], 1 ) = false
includes( [1..10), 10 ) = false
includes( [1..10], [4..6] ) = true
includes( [1..10], [1..5] ) = true
includes( (1..10], (1..5] ) = true
includes( [1..10], (1..10) ) = true
includes( [1..10), [5..10) ) = true
includes( [1..10], [1..10) ) = true
includes( [1..10], (1..10] ) = true
includes( [1..10], [1..10] ) = true
during( 5, [1..10] ) = true
during( 12, [1..10] ) = false
during( 1, [1..10] ) = true
during( 10, [1..10] ) = true
during( 1, (1..10] ) = false
during( 10, [1..10) ) = false
during( [4..6], [1..10] ) = true
during( [1..5], [1..10] ) = true
during( (1..5], (1..10] ) = true
during( (1..10), [1..10] ) = true
during( [5..10), [1..10) ) = true
during( [1..10), [1..10] ) = true
during( (1..10], [1..10] ) = true
during( [1..10], [1..10] ) = true
starts( 1, [1..10] ) = true
starts( 1, (1..10] ) = false
starts( 2, [1..10] ) = false
starts( [1..5], [1..10] ) = true
starts( (1..5], (1..10] ) = true
starts( (1..5], [1..10] ) = false
starts( [1..5], (1..10] ) = false
starts( [1..10], [1..10] ) = true
starts( [1..10), [1..10] ) = true
starts( (1..10), (1..10) ) = true
started by( [1..10], 1 ) = true
started by( (1..10], 1 ) = false
started by( [1..10], 2 ) = false
started by( [1..10], [1..5] ) = true
started by( (1..10], (1..5] ) = true
started by( [1..10], (1..5] ) = false
started by( (1..10], [1..5] ) = false
started by( [1..10], [1..10] ) = true
started by( [1..10], [1..10) ) = true
started by( (1..10), (1..10) ) = true
coincides( 5, 5 ) = true
coincides( 3, 4 ) = false
coincides( [1..5], [1..5] ) = true
coincides( (1..5), [1..5] ) = false
coincides( [1..5], [2..6] ) = false"""
for k, line in enumerate(T78.splitlines()):
    expr, res = line.rsplit(' = ', 1)
    v(f"t78-{k:03}", "Table 78", expr.strip(), res.strip())
# Table 74 string examples not yet covered
for k, (e, r) in enumerate([('starts with("foobar", "fo")', 'true'), ('ends with("foobar", "r")', 'true'),
                            ('lower case("aBc4")', '"abc4"'), ('substring("foobar", 3, 3)', '"oba"'),
                            ('string length("foo")', '3'), ('replace("banana", "a", "o")', '"bonono"'),
                            ('split("John Doe", "\\\\s")', '["John","Doe"]'), ('string join(["a","b","c"], "_and_")', '"a_and_b_and_c"'),
                            ('string join([])', '""'), ('substring after("foobar", "ob")', '"ar"')]):
    v(f"t74-{k:03}", "Table 74", e, r)
v("t-ms-eq", "Table 53", 'time("10:30:00.0001") = time("10:30:00.0002")', "true")
v("t-ms-lt", "Table 54", 'time("10:30:00.1") < time("10:30:00.2")', "true")
v("t-ms-dt", "Table 54", '@"2018-12-08T23:59:59.999" < @"2018-12-09T00:00:00"', "true")
import json, re
txt = json.dumps(V, indent=1, ensure_ascii=False)
txt = re.sub(r'"__RAW__([^"]*)"', r'\1', txt)
open('tests/vectors/semantic.json', 'w').write(txt + "\n")
print(len(V))

//! Test bindings for externally-defined functions (10.3.2.13.3).
//!
//! DMN leaves classpath configuration implementation-defined. This host
//! resolves `function(..) external {java: {class, method signature}}` to a
//! binding keyed `java:<class>.<signature without spaces>`. The bindings below
//! emulate the `java.lang` methods exercised by DMN TCK test 0076 using the
//! Java type mapping of Table 47 (`double`/`float`/`int`/`long` arguments are
//! converted from FEEL numbers; `double` results are converted back). Binary
//! floating point is used only here, because the Java signatures demand it.

use crate::HostFunctions;

fn num_arg(a: &str) -> Result<f64, String> {
    let t = a.trim();
    let t = t.strip_prefix("{\"@number\":\"").and_then(|x| x.strip_suffix("\"}")).unwrap_or(t);
    t.parse::<f64>().map_err(|_| format!("not a number: {a}"))
}

fn int_arg(a: &str) -> Result<i64, String> {
    let f = num_arg(a)?;
    if f.fract() != 0.0 {
        return Err(format!("not an integer: {a}"));
    }
    Ok(f as i64)
}

fn str_arg(a: &str) -> Result<String, String> {
    serde_json::from_str::<String>(a).map_err(|_| format!("not a string: {a}"))
}

fn num_out(f: f64) -> Result<String, String> {
    if !f.is_finite() {
        return Err("non-finite result".into());
    }
    // Java's Double.toString-equivalent shortest representation; FEEL-JSON
    // numbers must not use exponent notation for small magnitudes
    let s = format!("{f}");
    if s.contains('e') || s.contains('E') {
        return Ok(format!("{{\"@number\":\"{s}\"}}"));
    }
    Ok(s)
}

fn str_out(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn arity(args: &[String], n: usize) -> Result<(), String> {
    if args.len() == n {
        Ok(())
    } else {
        Err(format!("expected {n} arguments"))
    }
}

pub fn test_bindings() -> HostFunctions {
    let mut h = HostFunctions::new();
    h.add("java:java.lang.Math.max(double,double)", |a| {
        arity(a, 2)?;
        num_out(num_arg(&a[0])?.max(num_arg(&a[1])?))
    });
    h.add("java:java.lang.Math.max(float,float)", |a| {
        arity(a, 2)?;
        let x = num_arg(&a[0])? as f32;
        let y = num_arg(&a[1])? as f32;
        num_out(x.max(y) as f64).map(|_| format!("{}", x.max(y)))
    });
    h.add("java:java.lang.Math.max(int,int)", |a| {
        arity(a, 2)?;
        let (x, y) = (int_arg(&a[0])?, int_arg(&a[1])?);
        if x.abs() > i32::MAX as i64 || y.abs() > i32::MAX as i64 {
            return Err("int overflow".into());
        }
        Ok(x.max(y).to_string())
    });
    h.add("java:java.lang.Math.max(long,long)", |a| {
        arity(a, 2)?;
        Ok(int_arg(&a[0])?.max(int_arg(&a[1])?).to_string())
    });
    h.add("java:java.lang.Math.min(double,double)", |a| {
        arity(a, 2)?;
        num_out(num_arg(&a[0])?.min(num_arg(&a[1])?))
    });
    h.add("java:java.lang.Math.cos(double)", |a| {
        arity(a, 1)?;
        num_out(num_arg(&a[0])?.cos())
    });
    h.add("java:java.lang.Math.sin(double)", |a| {
        arity(a, 1)?;
        num_out(num_arg(&a[0])?.sin())
    });
    h.add("java:java.lang.Math.abs(double)", |a| {
        arity(a, 1)?;
        num_out(num_arg(&a[0])?.abs())
    });
    h.add("java:java.lang.Short.valueOf(short)", |a| {
        arity(a, 1)?;
        let x = int_arg(&a[0])?;
        if x < i16::MIN as i64 || x > i16::MAX as i64 {
            return Err("short overflow".into());
        }
        Ok(x.to_string())
    });
    h.add("java:java.lang.Byte.valueOf(byte)", |a| {
        arity(a, 1)?;
        let x = int_arg(&a[0])?;
        if x < i8::MIN as i64 || x > i8::MAX as i64 {
            return Err("byte overflow".into());
        }
        Ok(x.to_string())
    });
    h.add("java:java.lang.String.valueOf(char)", |a| {
        arity(a, 1)?;
        let s = str_arg(&a[0])?;
        if s.chars().count() != 1 {
            return Err("not a single char".into());
        }
        Ok(str_out(&s))
    });
    h.add("java:java.lang.Integer.valueOf(java.lang.String)", |a| {
        arity(a, 1)?;
        let s = str_arg(&a[0])?;
        let x: i32 = s.trim().parse().map_err(|_| "NumberFormatException".to_string())?;
        Ok(x.to_string())
    });
    h.add("java:java.lang.Float.valueOf(java.lang.String)", |a| {
        arity(a, 1)?;
        let s = str_arg(&a[0])?;
        let x: f32 = s.trim().parse().map_err(|_| "NumberFormatException".to_string())?;
        Ok(format!("{x}"))
    });
    h.add("java:java.lang.Double.valueOf(java.lang.String)", |a| {
        arity(a, 1)?;
        let s = str_arg(&a[0])?;
        let x: f64 = s.trim().parse().map_err(|_| "NumberFormatException".to_string())?;
        num_out(x)
    });
    h.add("java:java.lang.String.format(java.lang.String,[Ljava.lang.Object;)", |a| {
        if a.is_empty() {
            return Err("missing format".into());
        }
        let fmt = str_arg(&a[0])?;
        let mut out = String::new();
        let mut args = a[1..].iter();
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('s') => {
                        let v = args.next().ok_or("missing argument")?;
                        out.push_str(&serde_json::from_str::<String>(v).unwrap_or_else(|_| v.clone()));
                    }
                    Some('%') => out.push('%'),
                    _ => return Err("unsupported format".into()),
                }
            } else {
                out.push(c);
            }
        }
        Ok(str_out(&out))
    });
    h
}

//! What a script actually asks of its host.
//!
//! ```text
//! cargo run --release -p vpw-vbscript --example members -- table.vpx [more.vpx ...]
//! ```
//!
//! Walks the parsed tree and counts every `x.Member` and every bare name that
//! is not declared anywhere in the script. Between them that is the surface a
//! host has to implement, measured rather than guessed.

use std::collections::HashMap;
use vpw_vbscript::ast::*;

#[derive(Default)]
struct Tally {
    members: HashMap<String, usize>,
    reads: HashMap<String, usize>,
    declared: Vec<String>,
    handlers: HashMap<String, usize>,
}

fn main() {
    let mut t = Tally::default();
    for path in std::env::args().skip(1) {
        let code = if path.ends_with(".vpx") {
            let b = std::fs::read(&path).expect("could not read");
            vpin::vpx::from_bytes(&b)
                .expect("bad .vpx")
                .gamedata
                .code
                .string
        } else {
            String::from_utf8_lossy(&std::fs::read(&path).expect("could not read")).into_owned()
        };
        match vpw_vbscript::parser::parse(&code) {
            Ok(p) => walk_block(&p.body, &mut t),
            Err(e) => eprintln!("{path}: {e}"),
        }
    }

    let declared: std::collections::HashSet<String> =
        t.declared.iter().map(|s| s.to_lowercase()).collect();

    println!("=== members ({} distinct)", t.members.len());
    show(&t.members, 60);
    println!();
    println!("=== undeclared names read, i.e. asked of the host");
    let outside: HashMap<String, usize> = t
        .reads
        .iter()
        .filter(|(k, _)| !declared.contains(&k.to_lowercase()))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    show(&outside, 50);
    println!();
    println!("=== event handlers by suffix");
    show(&t.handlers, 40);
}

fn show(m: &HashMap<String, usize>, n: usize) {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (name, count) in v.into_iter().take(n) {
        println!("  {count:>5}  {name}");
    }
}

fn bump(m: &mut HashMap<String, usize>, k: &str) {
    *m.entry(k.to_string()).or_default() += 1;
}

fn walk_block(b: &[Stmt], t: &mut Tally) {
    for s in b {
        walk_stmt(s, t);
    }
}

fn walk_stmt(s: &Stmt, t: &mut Tally) {
    match &s.kind {
        StmtKind::Dim(names) => {
            for d in names {
                t.declared.push(d.name.to_string());
                if let Some(b) = &d.bounds {
                    for e in b {
                        walk_expr(e, t);
                    }
                }
            }
        }
        StmtKind::Const(items) => {
            for (n, e) in items {
                t.declared.push(n.to_string());
                walk_expr(e, t);
            }
        }
        StmtKind::ReDim { targets, .. } => {
            for (n, b) in targets {
                t.declared.push(n.to_string());
                for e in b {
                    walk_expr(e, t);
                }
            }
        }
        StmtKind::Assign { target, value, .. } => {
            walk_expr(target, t);
            walk_expr(value, t);
        }
        StmtKind::Call(e) => walk_expr(e, t),
        StmtKind::If {
            branches,
            else_body,
        } => {
            for (c, b) in branches {
                walk_expr(c, t);
                walk_block(b, t);
            }
            if let Some(b) = else_body {
                walk_block(b, t);
            }
        }
        StmtKind::For {
            var,
            from,
            to,
            step,
            body,
        } => {
            t.declared.push(var.to_string());
            walk_expr(from, t);
            walk_expr(to, t);
            if let Some(e) = step {
                walk_expr(e, t);
            }
            walk_block(body, t);
        }
        StmtKind::ForEach { var, seq, body } => {
            t.declared.push(var.to_string());
            walk_expr(seq, t);
            walk_block(body, t);
        }
        StmtKind::Do { cond, body } => {
            if let Some(c) = cond {
                walk_expr(&c.expr, t);
            }
            walk_block(body, t);
        }
        StmtKind::While { cond, body } => {
            walk_expr(cond, t);
            walk_block(body, t);
        }
        StmtKind::Select {
            subject,
            cases,
            default,
        } => {
            walk_expr(subject, t);
            for c in cases {
                for e in &c.tests {
                    walk_expr(e, t);
                }
                walk_block(&c.body, t);
            }
            if let Some(b) = default {
                walk_block(b, t);
            }
        }
        StmtKind::With { subject, body } => {
            walk_expr(subject, t);
            walk_block(body, t);
        }
        StmtKind::Proc(p) => {
            t.declared.push(p.name.to_string());
            for param in &p.params {
                t.declared.push(param.name.to_string());
            }
            if let Some((_, event)) = p.name.rsplit_once('_') {
                bump(&mut t.handlers, event);
            }
            walk_block(&p.body, t);
        }
        StmtKind::Class(c) => {
            t.declared.push(c.name.to_string());
            for f in &c.fields {
                t.declared.push(f.name.to_string());
            }
            for m in &c.members {
                t.declared.push(m.name.to_string());
                for param in &m.params {
                    t.declared.push(param.name.to_string());
                }
                walk_block(&m.body, t);
            }
        }
        StmtKind::Erase(names) => {
            for n in names {
                bump(&mut t.reads, n);
            }
        }
        StmtKind::Exit(_) | StmtKind::OnError { .. } | StmtKind::Nop => {}
    }
}

fn walk_expr(e: &Expr, t: &mut Tally) {
    match e {
        Expr::Ident(n) => bump(&mut t.reads, n),
        Expr::Member { base, name } => {
            bump(&mut t.members, name);
            walk_expr(base, t);
        }
        Expr::WithMember { name } => bump(&mut t.members, name),
        Expr::Index { base, args } => {
            walk_expr(base, t);
            for a in args.iter().flatten() {
                walk_expr(a, t);
            }
        }
        Expr::New(n) => bump(&mut t.reads, n),
        Expr::Unary { operand, .. } => walk_expr(operand, t),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, t);
            walk_expr(rhs, t);
        }
        _ => {}
    }
}

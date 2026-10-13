//! Headless command-line tool for robocad.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rc_mesh::stl::{self, StlFormat};
use rc_mesh::topology;

#[derive(Parser)]
#[command(name = "robocad-cli", version, about = "Headless robocad tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate a robocad project (all booleans) and export the printable result as STL.
    Export {
        project: PathBuf,
        output: PathBuf,
        /// Write ASCII STL instead of binary.
        #[arg(long)]
        ascii: bool,
    },
    /// Print size, volume and printability information about an STL file or a project.
    Info { file: PathBuf },
    /// Repair an STL file: merge near vertices, remove bad triangles, separate touching parts,
    /// fix orientation and fill holes.
    Repair {
        input: PathBuf,
        output: PathBuf,
        /// Do not fill holes.
        #[arg(long)]
        no_fill: bool,
        /// Also remove parts with fewer triangles than this.
        #[arg(long, default_value_t = 0)]
        min_part_triangles: usize,
    },
    /// Convert an STL file between binary and ASCII.
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// Write ASCII STL instead of binary.
        #[arg(long)]
        ascii: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { file } if is_project(&file) => project_info(&file),
        Command::Info { file } => info(&file),
        Command::Export {
            project,
            output,
            ascii,
        } => export(&project, &output, ascii),
        Command::Convert {
            input,
            output,
            ascii,
        } => convert(&input, &output, ascii),
        Command::Repair {
            input,
            output,
            no_fill,
            min_part_triangles,
        } => repair(&input, &output, !no_fill, min_part_triangles),
    }
}

fn load(path: &Path) -> Result<stl::StlFile> {
    stl::read_file(path).with_context(|| format!("failed to read {}", path.display()))
}

fn info(path: &Path) -> Result<()> {
    let start = Instant::now();
    let file = load(path)?;
    let elapsed = start.elapsed();
    let m = &file.mesh;
    let info = m.info();
    let topo = topology::analyze(m);
    let s = info.aabb.size();
    println!("file:        {}", path.display());
    println!(
        "format:      {:?} (loaded in {:.1} ms)",
        file.format,
        elapsed.as_secs_f64() * 1e3
    );
    if !file.name.is_empty() {
        println!("name:        {}", file.name);
    }
    println!("triangles:   {}", info.triangle_count);
    println!("vertices:    {}", info.vertex_count);
    println!("size:        {:.3} x {:.3} x {:.3} mm", s.x, s.y, s.z);
    println!(
        "bounds:      [{:.3}, {:.3}, {:.3}] .. [{:.3}, {:.3}, {:.3}]",
        info.aabb.min.x,
        info.aabb.min.y,
        info.aabb.min.z,
        info.aabb.max.x,
        info.aabb.max.y,
        info.aabb.max.z
    );
    println!("volume:      {:.3} mm³", info.volume);
    println!("area:        {:.3} mm²", info.area);
    println!("components:  {}", topo.components);
    println!(
        "watertight:  {}",
        if topo.is_watertight() { "yes" } else { "no" }
    );
    if !topo.is_clean_solid() || info.volume < 0.0 {
        println!("issues:");
        let items = [
            (topo.boundary_edges, "open (boundary) edges"),
            (topo.non_manifold_edges, "non-manifold edges"),
            (topo.non_manifold_vertices, "non-manifold vertices"),
            (topo.degenerate_triangles, "degenerate triangles"),
            (
                topo.inconsistent_edges,
                "edges with inconsistent orientation",
            ),
        ];
        for (n, what) in items.iter().filter(|(n, _)| *n > 0) {
            println!("  - {n} {what}");
        }
        if info.volume < 0.0 {
            println!("  - mesh is inside out (negative volume)");
        }
    }
    for w in &file.warnings {
        println!("warning:     {w}");
    }
    Ok(())
}

fn convert(input: &Path, output: &Path, ascii: bool) -> Result<()> {
    let file = load(input)?;
    let format = if ascii {
        StlFormat::Ascii
    } else {
        StlFormat::Binary
    };
    let name = if file.name.is_empty() {
        "robocad".to_string()
    } else {
        file.name.clone()
    };
    stl::write_file(&file.mesh, output, format, &name)
        .with_context(|| format!("failed to write {}", output.display()))?;
    println!(
        "wrote {} triangles to {} ({format:?})",
        file.mesh.triangle_count(),
        output.display()
    );
    Ok(())
}

fn repair(input: &Path, output: &Path, fill: bool, min_part_triangles: usize) -> Result<()> {
    use rc_mesh::repair::{RepairOptions, repair};
    let file = load(input)?;
    let opts = RepairOptions {
        fill_holes: fill,
        min_component_triangles: min_part_triangles,
        ..RepairOptions::FULL
    };
    let (fixed, report) = repair(&file.mesh, &opts);
    let lines = report.summary();
    if lines.is_empty() {
        println!("nothing to repair");
    } else {
        for l in lines {
            println!("- {l}");
        }
    }
    stl::write_file(&fixed, output, StlFormat::Binary, "robocad repaired")
        .with_context(|| format!("failed to write {}", output.display()))?;
    let after = report.after;
    println!(
        "wrote {} ({} triangles) — watertight: {}",
        output.display(),
        fixed.triangle_count(),
        if after.is_watertight() { "yes" } else { "no" }
    );
    Ok(())
}

fn is_project(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(rc_doc::project::EXTENSION))
}

fn load_project(path: &Path) -> Result<rc_doc::project::Project> {
    rc_doc::project::load(path).with_context(|| format!("failed to open {}", path.display()))
}

fn project_info(path: &Path) -> Result<()> {
    let p = load_project(path)?;
    let doc = &p.document;
    println!("project:     {}", path.display());
    println!(
        "objects:     {} ({} at top level)",
        doc.nodes.len(),
        doc.roots.len()
    );
    for (id, depth) in doc.walk() {
        let Some(n) = doc.get(id) else { continue };
        let kind = match &n.kind {
            rc_doc::NodeKind::Imported { source, .. } => format!("mesh from {source}"),
            rc_doc::NodeKind::Primitive(p) => p.kind().name().to_lowercase(),
            rc_doc::NodeKind::Group { .. } => "group".into(),
        };
        let role = if n.role == rc_doc::Role::Hole {
            ", hole"
        } else {
            ""
        };
        println!("  {}{} ({kind}{role})", "  ".repeat(depth), n.name);
    }
    let b = doc.scene_bounds();
    let s = b.size();
    println!("size:        {:.3} x {:.3} x {:.3} mm", s.x, s.y, s.z);
    Ok(())
}

fn export(project: &Path, output: &Path, ascii: bool) -> Result<()> {
    let p = load_project(project)?;
    let roots = p.document.roots.clone();
    let mut cache = rc_doc::eval::SolidCache::default();
    let r = rc_doc::eval::export_mesh(&p.document, &roots, &mut cache);
    for w in &r.warnings {
        eprintln!("warning: {w}");
    }
    anyhow::ensure!(
        !r.mesh.is_empty(),
        "nothing to export: no visible solid objects"
    );
    let format = if ascii {
        StlFormat::Ascii
    } else {
        StlFormat::Binary
    };
    stl::write_file(&r.mesh, output, format, "robocad")
        .with_context(|| format!("failed to write {}", output.display()))?;
    let watertight = topology::analyze(&r.mesh).is_watertight();
    println!(
        "wrote {} ({} triangles, {:.3} mm³) — watertight: {}",
        output.display(),
        r.mesh.triangle_count(),
        r.mesh.volume(),
        if watertight { "yes" } else { "no" }
    );
    Ok(())
}

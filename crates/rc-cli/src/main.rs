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
    /// Print size, volume and printability information about an STL file.
    Info { file: PathBuf },
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
        Command::Info { file } => info(&file),
        Command::Convert {
            input,
            output,
            ascii,
        } => convert(&input, &output, ascii),
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

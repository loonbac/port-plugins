//! Punto de entrada del ejecutable `port-herdr`.
//!
//! Se encarga únicamente de recolectar los argumentos del proceso y delegar
//! la ejecución en la biblioteca, convirtiendo el resultado en el código
//! de salida correspondiente sin generar dependencias en tiempo de ejecución.

use port_herdr::run;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exit_code = run(args.iter().map(String::as_str));
    std::process::exit(exit_code);
}

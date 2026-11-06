# Legacy nix-shell and nix develop share the exact locked environment.
(builtins.getFlake (toString ./.)).devShells.x86_64-linux.default

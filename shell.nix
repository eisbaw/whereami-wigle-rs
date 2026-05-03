(import
  (let lock = builtins.fromJSON (builtins.readFile ./flake.lock); in
    fetchTarball {
      url = "https://github.com/edolstra/flake-compat/archive/35bb57c0c8d8b4f45571e8f20c7571aa6df0a9ae.tar.gz";
      sha256 = "1prd9b1xx8c0sfwnyzkspplz28b0zvmph7m3chatbrb8kfns6368";
    })
  { src = ./.; }
).shellNix

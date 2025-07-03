@0x9663f4dd604afa36;

interface ControlPlane {
  apply @0 (file: Text, name: Text) -> (response: ApplyResponse);
  monitor @1 () -> (response: List(AppData));
}

struct AppData {
    id @0 :Text;
    memory @1 :Float64;
    cpu @2 :Float64;
    uptime @3 :Float64;
}


struct ApplyResponse {
union {
  err @0 :Text;
  ok @1 :Void;
  }
}

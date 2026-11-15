// Requires `femp build --target wasm` to have produced the bindings.
import { VestingWeb } from "../build/wasm/web/vesting_web.js";
import { GuestAbort } from "../build/wasm/web/host.js";

const ERRORS = {
  1: "The total must be greater than zero.",
  2: "The duration must be greater than zero.",
  3: "The cliff cannot be longer than the duration.",
  4: "The schedule ends beyond the largest supported timestamp.",
  5: "A claim must be greater than zero.",
  6: "More than the claimable amount was requested.",
  7: "The amount already claimed is larger than the total.",
};

const form = document.querySelector("#schedule");
const vested = document.querySelector("#vested");
const claimable = document.querySelector("#claimable");
const error = document.querySelector("#error");

const vesting = await VestingWeb.load();

function field(name) {
  return BigInt(form.elements[name].value.trim() || "0");
}

function show(message) {
  error.textContent = message;
  error.hidden = message === "";
  if (message !== "") {
    vested.textContent = "–";
    claimable.textContent = "–";
  }
}

function update() {
  try {
    const schedule = [field("total"), field("start"), field("cliff"), field("duration")];
    vested.textContent = vesting.vestedAt(...schedule, field("now")).toString();
    claimable.textContent = vesting
      .claimableAt(...schedule, field("claimed"), field("now"))
      .toString();
    show("");
  } catch (failure) {
    if (failure instanceof GuestAbort) {
      show(ERRORS[failure.code] ?? `Error ${failure.code}`);
    } else if (failure instanceof SyntaxError) {
      show("Enter whole numbers only.");
    } else {
      throw failure;
    }
  }
}

form.addEventListener("input", update);
update();

//! Published input control: reads a smoothed value or trigger pulse from the voice.

use crate::catalog::{Category, NodeTypeDef, ParamDef, ParamKind};
use crate::consts::RESERVED_INPUTS;
use crate::node::{Node, NodeInit, NodeOut, SampleCtx};

static PARAMS: [ParamDef; 1] = [ParamDef {
    key: "name",
    kind: ParamKind::Text { default: "" },
    unit: "",
    label_en: "Name",
    label_fr: "Nom",
    modulatable: false,
    smooth: false,
}];

/// Catalog definition of the published input control node.
pub static DEF: NodeTypeDef = NodeTypeDef {
    type_name: "input",
    category: Category::Control,
    label_en: "Input",
    label_fr: "Entrée",
    audio_in: false,
    audio_out: false,
    control_out: true,
    uses_curve: false,
    hidden: false,
    params: &PARAMS,
    create,
};

fn resolve(init: &NodeInit<'_>) -> Option<usize> {
    let name = init.texts.first()?.as_deref()?;
    init.inputs.iter().position(|input| input.name == name).or_else(|| {
        RESERVED_INPUTS.iter().position(|(reserved, _)| *reserved == name)
            .map(|index| init.inputs.len() + index)
    })
}

fn create(init: &NodeInit<'_>) -> Box<dyn Node> {
    Box::new(InputNode { index: resolve(init) })
}

struct InputNode {
    index: Option<usize>,
}

impl Node for InputNode {
    fn process_sample(&mut self, ctx: &SampleCtx<'_>, _input: (f64, f64), _params: &[f64]) -> NodeOut {
        let value = self.index.and_then(|index| ctx.inputs.get(index)).copied().unwrap_or(0.0);
        NodeOut { l: 0.0, r: 0.0, value }
    }

    fn configure(&mut self, init: &NodeInit<'_>) {
        self.index = resolve(init);
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use crate::{consts::BLOCK, Voice};

    fn patch(name: &str) -> String {
        format!(r#"{{"haa":1,"id":"input-test","title":"Input test",
          "inputs":[{{"name":"level","kind":"continuous","min":0,"max":1,"default":0,"smooth":0.05}}],
          "graph":{{"duration":1,"mode":"live","nodes":[
            {{"id":"input","type":"input","params":{{"name":"{name}"}}}},
            {{"id":"osc","type":"oscillator","params":{{"frequency":220}}}},
            {{"id":"gain","type":"gain","params":{{"gain":0}}}},
            {{"id":"out","type":"output"}}],
            "connections":[
              {{"id":"control","from":"input","to":"gain","target":"gain","amount":1}},
              {{"id":"audio1","from":"osc","to":"gain","target":"audio","amount":1}},
              {{"id":"audio2","from":"gain","to":"out","target":"audio","amount":1}}]}}}}"#)
    }

    fn load(name: &str) -> (Voice, usize) {
        let mut voice = Voice::new(48_000.0, 0);
        let outcome = voice.load(&patch(name), false);
        assert!(outcome.playable, "{:?}", outcome.issues);
        let pos = voice.node_ids().iter().position(|id| id == "input").unwrap();
        (voice, pos)
    }

    #[test]
    fn input_follows_set_input_through_the_smoother() {
        let (mut voice, pos) = load("level");
        let index = voice.input_index("level").unwrap();
        voice.set_input(index, 1.0);
        let mut left = [0.0; BLOCK];
        let mut right = [0.0; BLOCK];
        voice.process(&mut left, &mut right);
        assert!(voice.trace()[pos] > 0.0 && voice.trace()[pos] < 1.0);
        for _ in 1..(24_000 / BLOCK) {
            voice.process(&mut left, &mut right);
        }
        assert!(voice.trace()[pos] > 0.95);
    }

    #[test]
    fn reserved_input_resolves_after_header_inputs() {
        let (mut voice, pos) = load("_doppler");
        assert_eq!(voice.input_index("_doppler"), Some(3));
        voice.process(&mut [0.0; BLOCK], &mut [0.0; BLOCK]);
        assert_eq!(voice.trace()[pos], 1.0);
    }
}

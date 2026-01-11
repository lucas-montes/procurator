use std::path::Path;

/// Marker trait for all parser states
pub trait ParserState: Sized {}

/// Trait for states that can advance to the next stage
pub trait Advance: ParserState {
    type Next: ParserState;

    fn advance(self) -> Self::Next;
}

/// Trait for terminal states - no more advancement possible
pub trait Terminal: ParserState {}

/// Trait for states that can be cached
pub trait Cacheable: ParserState {
    fn save(&self, path: &Path) -> std::io::Result<()>;
    fn load(path: &Path) -> std::io::Result<Self>;
}

/// Trait for states that can be displayed
pub trait Printable: ParserState {
    fn print(&self);
}

/// Trait for initial states - can start a pipeline
pub trait Initial: ParserState {}

/// Core Parser trait - all parsers must implement this
pub trait Parser<S: ParserState>: Sized + From<S> {
    /// Get a reference to the current state
    fn state(&self) -> &S;
}


/// Pipeline wrapper that tracks the stage and parser type
#[derive(Debug)]
pub struct Pipeline<P, S, Stage>
where
    S: ParserState,
    P: Parser<S>,
    Stage: PipelineStage,
{
    parser: P,
    _state: std::marker::PhantomData<S>,
    _stage: std::marker::PhantomData<Stage>,
}

impl<P, S, Stage> Pipeline<P, S, Stage>
where
    S: ParserState,
    P: Parser<S>,
    Stage: PipelineStage,
{
    pub fn new(parser: P) -> Self {
        Self {
            parser,
            _state: std::marker::PhantomData,
            _stage: std::marker::PhantomData,
        }
    }

    pub fn state(&self) -> &S {
        self.parser.state()
    }

    pub fn into_parser(self) -> P {
        self.parser
    }
}

/// Stage marker traits - define the pipeline phases
pub trait PipelineStage: sealed::Sealed {}

mod sealed {
    pub trait Sealed {}
}

/// Discovery stage - initial file/data discovery
pub struct Discovery;
impl sealed::Sealed for Discovery {}
impl PipelineStage for Discovery {}

/// Analysis stage - analyzing discovered data
pub struct Analysis;
impl sealed::Sealed for Analysis {}
impl PipelineStage for Analysis {}

/// Transformation stage - transforming into final representation
pub struct Transformation;
impl sealed::Sealed for Transformation {}
impl PipelineStage for Transformation {}

/// Completion stage - final terminal state
pub struct Completion;
impl sealed::Sealed for Completion {}
impl PipelineStage for Completion {}

/// Advance from Discovery to Analysis
impl<P, S> Pipeline<P, S, Discovery>
where
    S: Advance + From<P>,
    P: Parser<S>,
{
    pub fn analyze<NextP>(self) -> Pipeline<NextP, S::Next, Analysis>
    where
        NextP: Parser<S::Next>,
        S::Next: Into<Analysis>,
    {
        let next_state = Into::<S>::into(self.parser).advance();
        Pipeline::new(NextP::from(next_state))
    }
}

/// Advance from Analysis to Transformation
impl<P, S> Pipeline<P, S, Analysis>
where
    S: Advance + From<P>,
    P: Parser<S>,
{
    pub fn transform<NextP>(self) -> Pipeline<NextP, S::Next, Transformation>
    where
        NextP: Parser<S::Next>,
        S::Next: Into<Transformation>,
    {
        let next_state = Into::<S>::into(self.parser).advance();
        Pipeline::new(NextP::from(next_state))
    }
}

/// Advance from Transformation to Completion
impl<P, S> Pipeline<P, S, Transformation>
where
    S: Advance + From<P>,
    P: Parser<S>,
{
    pub fn complete<NextP>(self) -> Pipeline<NextP, S::Next, Completion>
    where
        NextP: Parser<S::Next>,
        S::Next: Terminal + Into<Completion>,
    {
        let next_state = Into::<S>::into(self.parser).advance();
        Pipeline::new(NextP::from(next_state))
    }
}


/// Convenience methods for Printable states (available at any stage)
impl<P, S, Stage> Pipeline<P, S, Stage>
where
    S: Printable,
    P: Parser<S>,
    Stage: PipelineStage,
{
    pub fn print(&self) {
        self.state().print()
    }
}

/// Convenience methods for Cacheable states (available at any stage)
impl<P, S, Stage> Pipeline<P, S, Stage>
where
    S: Cacheable,
    P: Parser<S>,
    Stage: PipelineStage,
{
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        self.state().save(path)
    }

    pub fn load(path: &Path) -> std::io::Result<Self>
    where
        P: Parser<S>,
    {
        S::load(path).map(P::from).map(Pipeline::new)
    }
}

/// Helper trait to create initial pipelines
pub trait BeginPipeline: Initial + ParserState + Sized {
    fn begin<P, Stage>(self) -> Pipeline<P, Self, Stage>
    where
        P: Parser<Self>,
        Stage: PipelineStage,
        Self: Into<Stage>,
    {
        Pipeline::new(P::from(self))
    }
}

impl<T: Initial + ParserState> BeginPipeline for T {}

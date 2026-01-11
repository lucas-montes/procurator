// A project is a collection of repos. It is used to  group services together to be able to launch
// them all at once, or individually with any other dependency required such as databases. The
// flake resulting from it will be similar to the ones from the repos, with the difference that it
// will probably need all the dependencies of the repos to be built beforehand.

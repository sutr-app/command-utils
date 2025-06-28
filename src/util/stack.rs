use std::fmt::Debug;

/// Types of operations executed on the stack
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum Operation<T> {
    Push(T),
    Pop,
}

/// Stack implementation that maintains operation history
///
/// T: Type of elements stored in the stack
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StackWithHistory<T: Clone + Debug + serde::Serialize> {
    /// Current state of the stack
    current_state: Vec<T>,
    /// Operation history
    #[serde(skip)]
    history: Vec<Operation<T>>,
    /// Initial state of the stack
    #[serde(skip)]
    initial_state: Vec<T>,
}
impl<T: Clone + Debug + serde::Serialize> Default for StackWithHistory<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Debug + serde::Serialize> StackWithHistory<T> {
    /// Creates a new empty stack
    pub fn new() -> Self {
        Self {
            current_state: Vec::new(),
            history: Vec::new(),
            initial_state: Vec::new(),
        }
    }

    /// Creates a new empty stack
    pub fn new_with(initial: Vec<T>) -> Self {
        Self {
            current_state: initial.clone(),
            history: Vec::new(),
            initial_state: initial,
        }
    }

    /// Creates a new stack with the specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            current_state: Vec::with_capacity(capacity),
            history: Vec::new(),
            initial_state: Vec::new(),
        }
    }

    /// Adds an element to the stack
    pub fn push(&mut self, item: T) {
        self.current_state.push(item.clone());
        self.history.push(Operation::Push(item));
    }

    /// Removes and returns the top element from the stack
    pub fn pop(&mut self) -> Option<T> {
        let popped = self.current_state.pop();
        if popped.is_some() {
            self.history.push(Operation::Pop);
        }
        popped
    }
    pub fn last(&self) -> Option<&T> {
        self.current_state.last()
    }

    /// Returns a reference to the current stack state
    pub fn snapshot(&self) -> &[T] {
        &self.current_state
    }

    /// Returns a reference to the operation history
    pub fn history(&self) -> &[Operation<T>] {
        &self.history
    }

    /// Calculates and returns the state after going back a specified number of operations
    pub fn state_before_operations(&self, n: usize) -> Vec<T> {
        if n == 0 {
            return self.current_state.clone();
        }
        if n >= self.history.len() {
            return self.initial_state.clone();
        }

        let mut past_state = self.initial_state.clone();

        // Reconstruct the state using the history excluding the specified number of operations
        let history_length = self.history.len();
        let operations_to_apply = &self.history[0..(history_length - n)];

        for op in operations_to_apply {
            match op {
                Operation::Push(item) => past_state.push(item.clone()),
                Operation::Pop => {
                    past_state.pop();
                }
            }
        }

        past_state
    }

    /// Creates a new StackWithHistory representing the state before a specified number of operations
    ///
    /// Similar to `state_before_operations` but returns a complete StackWithHistory instance
    /// instead of just the state vector. The returned stack contains the history up to the
    /// specified point.
    pub fn stack_before_operations(&self, n: usize) -> Self {
        if n == 0 {
            // Simply clone the current stack if no operations to revert
            return self.clone();
        }

        // Calculate how many operations to keep in history
        let history_to_keep = if n >= self.history.len() {
            0 // Keep no history if reverting all operations
        } else {
            self.history.len() - n
        };

        // Get the state vector before n operations
        let past_state = self.state_before_operations(n);

        // Create a new stack with the past state and partial history
        Self {
            current_state: past_state,
            history: self.history[0..history_to_keep].to_vec(),
            initial_state: self.initial_state.clone(),
        }
    }

    /// Returns the length of the stack
    pub fn len(&self) -> usize {
        self.current_state.len()
    }

    /// Checks if the stack is empty
    pub fn is_empty(&self) -> bool {
        self.current_state.is_empty()
    }

    /// Returns the number of operations in the history
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Clears the history (maintains the current state)
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Clears both the stack and history
    pub fn clear_all(&mut self) {
        self.current_state.clear();
        self.history.clear();
    }

    /// Returns a reference to the top element without removing it
    pub fn peek(&self) -> Option<&T> {
        self.current_state.last()
    }

    /// Returns a JSON Pointer (RFC-6901) representation of the current stack state
    ///
    /// The stack is interpreted as a JSON Pointer path, with each element on the stack
    /// representing a path segment.
    ///
    /// # Example
    ///
    /// A stack with elements ["foo", "bar"] would produce the JSON Pointer "/foo/bar"
    pub fn json_pointer(&self) -> String {
        let mut result = String::new();

        for item in &self.current_state {
            result.push('/');

            // Convert the item to a string representation
            let str_repr = match serde_json::to_value(item) {
                Ok(serde_json::Value::String(s)) => s,
                Ok(value) => value.to_string(),
                Err(_) => format!("{item:?}"),
            };

            // According to RFC-6901, '~' must be encoded as '~0' and '/' as '~1'
            let segment = str_repr.replace('~', "~0").replace('/', "~1");
            result.push_str(&segment);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use std::vec;

    use super::*;

    #[test]
    fn test_push_and_pop() {
        let mut stack = StackWithHistory::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.snapshot(), &[1]);
        assert_eq!(stack.pop(), Some(1));
        assert!(stack.is_empty());
    }

    #[test]
    fn test_history() {
        let mut stack = StackWithHistory::<i32>::new();
        stack.push(10);
        stack.push(20);
        stack.pop();
        stack.push(30);

        // Check history
        assert_eq!(stack.history_len(), 4);

        // Current state [10, 30]
        assert_eq!(stack.snapshot(), &[10, 30]);

        // State 1 operation ago [10]
        assert_eq!(stack.state_before_operations(1), vec![10]);

        // State 2 operations ago [10, 20]
        assert_eq!(stack.state_before_operations(2), vec![10, 20]);

        // State 3 operations ago [10]
        assert_eq!(stack.state_before_operations(3), vec![10]);

        // State 4 operations ago (empty)
        assert_eq!(stack.state_before_operations(4), Vec::<i32>::new());

        // When going back more operations than in history
        assert_eq!(stack.state_before_operations(10), Vec::<i32>::new());
    }

    #[test]
    fn test_new_with() {
        let stack = StackWithHistory::new_with(vec![10, 20, 30]);

        // Verify that the initial state is correctly set
        assert_eq!(stack.snapshot(), &[10, 20, 30]);

        // Verify that the history is empty when initial values are provided
        assert_eq!(stack.history_len(), 0);

        // state_before_operations should consider the initial values
        assert_eq!(stack.state_before_operations(0), vec![10, 20, 30]);
    }

    #[test]
    fn test_initial_state_preservation() {
        // Create a stack with an initial state
        let initial_state = vec![10, 20, 30];
        let initial_clone = initial_state.clone();
        let mut stack = StackWithHistory::new_with(initial_state);

        // Perform some operations
        stack.push(40);
        stack.push(50);
        assert_eq!(stack.pop(), Some(50));
        stack.push(60);
        stack.push(70);
        assert_eq!(stack.pop(), Some(70));
        assert_eq!(stack.pop(), Some(60));

        // Verify that the original vector is unchanged
        assert_eq!(initial_clone, vec![10, 20, 30]);

        // Verify the initial state part of the stack itself
        let current = stack.snapshot();
        assert_eq!(current[0..3], [10, 20, 30]);

        // Verify state_before_operations
        // Current state [10, 20, 30, 40]
        assert_eq!(stack.snapshot(), &[10, 20, 30, 40]);

        // State 1 operation ago [10, 20, 30, 40, 60]
        assert_eq!(stack.state_before_operations(1), vec![10, 20, 30, 40, 60]);

        // State 2 operations ago [10, 20, 30, 40, 60, 70]
        assert_eq!(
            stack.state_before_operations(2),
            vec![10, 20, 30, 40, 60, 70]
        );

        // State 3 operations ago [10, 20, 30, 40, 60]
        assert_eq!(stack.state_before_operations(3), vec![10, 20, 30, 40, 60]);

        // State 4 operations ago [10, 20, 30, 40]
        assert_eq!(stack.state_before_operations(4), vec![10, 20, 30, 40]);

        // State 5 operations ago [10, 20, 30, 40, 50]
        assert_eq!(stack.state_before_operations(5), vec![10, 20, 30, 40, 50]);

        // State 6 operations ago [10, 20, 30, 40]
        assert_eq!(stack.state_before_operations(6), vec![10, 20, 30, 40]);

        // State 7 operations ago [10, 20, 30]
        assert_eq!(stack.state_before_operations(7), vec![10, 20, 30]);

        // Verify that going back more than the history length returns the initial state
        assert_eq!(stack.state_before_operations(10), vec![10, 20, 30]);
    }

    #[test]
    fn test_json_pointer() {
        // Test with basic string values
        let mut stack = StackWithHistory::new();
        stack.push("foo");
        stack.push("bar");

        // Should produce "/foo/bar"
        assert_eq!(stack.json_pointer(), "/foo/bar");

        // Test with values containing special characters
        let mut special_stack = StackWithHistory::new();
        special_stack.push("~tilde");
        special_stack.push("/slash");

        // Should encode ~ as ~0 and / as ~1
        assert_eq!(special_stack.json_pointer(), "/~0tilde/~1slash");

        // Test with integers
        let mut int_stack = StackWithHistory::new();
        int_stack.push(42);
        int_stack.push(100);

        assert_eq!(int_stack.json_pointer(), "/42/100");

        // Test with empty stack
        let empty_stack = StackWithHistory::<String>::new();
        assert_eq!(empty_stack.json_pointer(), "");

        // Test with initial values
        let init_stack = StackWithHistory::new_with(vec!["a", "b", "c"]);
        assert_eq!(init_stack.json_pointer(), "/a/b/c");
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_json_pointer_with_json_values() {
        use serde_json::json;

        let mut stack = StackWithHistory::new();

        // Test with string JSON values
        stack.push(json!("foo"));
        stack.push(json!("bar"));
        assert_eq!(stack.json_pointer(), "/foo/bar");

        // Clear and test with number values
        stack.clear_all();
        stack.push(json!(42));
        stack.push(json!(3.14));
        assert_eq!(stack.json_pointer(), "/42/3.14");

        // Test with nested objects and arrays
        stack.clear_all();
        stack.push(json!({"name": "John"}));
        stack.push(json!([1, 2, 3]));
        // must be careful with the representation of objects (should push separately to stack to get the correct representation)
        assert_eq!(stack.json_pointer(), "/{\"name\":\"John\"}/[1,2,3]"); // Object representation may vary

        // Test with null and boolean values
        stack.clear_all();
        stack.push(json!(null));
        stack.push(json!(true));
        stack.push(json!(false));
        assert_eq!(stack.json_pointer(), "/null/true/false");

        // Test with mixed values
        stack.clear_all();
        stack.push(json!("path"));
        stack.push(json!(123));
        stack.push(json!({"id": 456}));
        assert_eq!(stack.json_pointer(), "/path/123/{\"id\":456}"); // Object representation may vary
    }

    #[test]
    fn test_stack_before_operations() {
        // Setup original stack with initial state and operations
        let mut original_stack = StackWithHistory::new_with(vec![10, 20, 30]);
        original_stack.push(40);
        original_stack.push(50);
        original_stack.pop(); // pop 50
        original_stack.push(60);

        // Current state should be [10, 20, 30, 40, 60]
        assert_eq!(original_stack.snapshot(), &[10, 20, 30, 40, 60]);
        assert_eq!(original_stack.history_len(), 4); // 4 operations: push, push, pop, push

        // Revert 1 operation (removing the push(60))
        let reverted_stack = original_stack.stack_before_operations(1);

        // Check state and history
        assert_eq!(reverted_stack.snapshot(), &[10, 20, 30, 40]);
        assert_eq!(reverted_stack.history_len(), 3); // Should have 3 operations

        // Perform the same operation on the reverted stack
        let mut reverted_stack = reverted_stack;
        reverted_stack.push(60);

        // After performing the same operation, both stacks should be identical
        assert_eq!(reverted_stack.snapshot(), original_stack.snapshot());
        assert_eq!(reverted_stack.history_len(), 4);

        // Test with multiple operations
        let mut original_stack = StackWithHistory::new();
        original_stack.push(1);
        original_stack.push(2);
        original_stack.push(3);
        original_stack.pop(); // pop 3
        original_stack.push(4);
        original_stack.push(5);

        // Revert to the middle (after push(2))
        let mut reverted_stack = original_stack.stack_before_operations(4);
        assert_eq!(reverted_stack.snapshot(), &[1, 2]);
        assert_eq!(reverted_stack.history_len(), 2);

        // Perform same sequence of operations
        reverted_stack.push(3);
        reverted_stack.pop(); // pop 3
        reverted_stack.push(4);
        reverted_stack.push(5);

        // Results should match
        assert_eq!(reverted_stack.snapshot(), original_stack.snapshot());
        assert_eq!(reverted_stack.history_len(), 6);

        // Test with full history rollback
        let reverted_to_initial = original_stack.stack_before_operations(6);
        assert_eq!(reverted_to_initial.snapshot(), Vec::<i32>::new().as_slice());
        assert_eq!(reverted_to_initial.history_len(), 0);

        // Starting from initial state, perform all operations again
        let mut rebuilt_stack = reverted_to_initial;
        rebuilt_stack.push(1);
        rebuilt_stack.push(2);
        rebuilt_stack.push(3);
        rebuilt_stack.pop(); // pop 3
        rebuilt_stack.push(4);
        rebuilt_stack.push(5);

        // Results should match
        assert_eq!(rebuilt_stack.snapshot(), original_stack.snapshot());
        assert_eq!(rebuilt_stack.history_len(), 6);
    }
}

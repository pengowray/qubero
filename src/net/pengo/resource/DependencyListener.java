/**
 * DependencyListener.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

public interface DependencyListener
{
    
    /*
    
    * SuperSource for SINK,  Exclusive source for next node
     \
      * source (and supersource) for SINK (and sink for above)..
     / \
    *   * SINK
    
     parent = subscriber / listener
     dependency = subscription / publication / feed / resource
    
    */
    
    // resource's methods
    
    // weak dependencies
    // gc
    // prevent circular deps
    
    //fixme: sort this out into several classes/interfaces
    
    /*
    
    Informational link data (for display at this time):
     - source/sink
     - holo/mero (made up from/property)
     - switchable dependency
     - instance of
     - copy of
     - etc
    
    Links: (for preventing circularity)
     - sources
     - sinks
    
    Ports: (for setting/changing links)
     - inputPorts (PortedResource)
     - outputPorts (PortedResource)
    
    Transactions: (to allow multiple changes at once, e.g. that would otherwise give errors along the way)
     -
    
    Ordering and optimisation:
     - order independence: a(), b() == b(), a()
     - skippable commands: b(), a() == a()
     - compactable/equivilance: a(), b() == c()
     - dependant a() == b() until someone listens
     - security related pathway.. if b is security data in a(b).. output is also security data
     - IO related (e.g. original source from keyboard)
     - may lose events.. must be polled
     - causes state change
     - noop: a() == ""
     - debug only: (noop when compiled)
     - assertion (may be a noop when compiled)
     - Hard Real Time requirementshai
    
    Modification via Resource:
     - getSourcePorts
    
    Modification via SourcePort:
     - setSource
     - canUseSource(source)
    
     */
    
    /*
    
    test case:
    
    array
     - SourceField: "Length",
        - contraints: type contraint: Integer
        - Source:
           - Integer (a resource)
               Sinks: Integer's properties
    
    
    
     */
    
    public interface SourceField { // or SourcePort or SinkHole, or Field
	
    }
    
    public interface Sink {
	public void addSource();
	public void removeSource();
	
	public void getSourcePorts();
	
	public void getSources();
	public void getExclusiveSources();
	public void getSuperSources();
	public boolean hasSource(Resource r);
	public boolean hasInheritedSource(Resource r);

	public void sourceReplaced();
	public void sourceModified();
	public void sourceRemoved();
    }
    
    public interface Source {
	public void addSink();
	public void removeSink();
	public void getSinks();
	public void getSubSinks();
	
    }
    
    public void addConstraint();
    public void removeConstraint();
    public void getConstraints();
    
    
    public void addSourceListener();
    public void sourceAdded();
    public void sourceRemoved();
	
    // which is higher?
    public int compare();
    
}



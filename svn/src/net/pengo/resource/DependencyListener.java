/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * DependencyListener.java
 *
 * @author  Peter Halasz
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
     dependency = subscription / publication / feed / resource / trigger  
    
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
     - switchable dependency (?)
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
     - Hard Real Time requirements
    
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



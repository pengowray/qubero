/**
 * Link.java
 *
 * @author  Peter Halasz
 */

package net.pengo.dependency;

import net.pengo.resource.*;


/*

    Informational link data (just for display, at least for now):
     - source/sink
     - holo/mero (made up from/property)
     - switchable dependency (?)
     - instance of
     - copy of
     - etc
 

 */

public interface InfoLink
{
    public String getVerbA2B(); // the name of this sort of link between node A and B (e.g. A is "parent of" B)
    public String getVerbB2A(); // (e.g. B is "child of" A)
    public Resource getNodeA();
    public Resource getNodeB();
}


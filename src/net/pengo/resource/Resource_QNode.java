/**
 * QNode, for Qubero Node.
 * A node that can tell you what its sources are, and how many sinks it's got (for now)
 * 
 * Now rolled into Resource
 *
 * @author Peter Halasz
 */


package net.pengo.resource;

public interface Resource_QNode
{
    //public InfoLink[] getInfoLinkList();
    //public DepLink[] getDepLinkList();
    
    public Resource[] getSources();
    
    // temporary interface. useful for telling if this qnode is "owned" by its sink, or independent, or multiply-used
    public int getSinkCount();
    public void addSink(Resource sink);
    public void removeSink(Resource sink);
    
    // listener stuff? etc etc
    
}



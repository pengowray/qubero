/**
 * QNode, for Qubero Node. 
 * A node that can tell you what its sources are, and how many sinks it's got (for now)
 *
 * @author Peter Halasz
 */


package net.pengo.dependency;

public interface QNode
{
    //public InfoLink[] getInfoLinkList();
    //public DepLink[] getDepLinkList();
    
    public QNode[] getSources();
    
    // temporary interface. useful for telling if this qnode is "owned" by its sink, or independent, or multiply-used
    public int getSinkCount();
    public void addSink(QNode sink);
    public void removeSink(QNode sink);
    
    // listener stuff? etc etc
    
}



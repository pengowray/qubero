import javax.swing.*;
/*
 * MoojNode.java
 *
 * 
 *
 * Created on 1 August 2002, 13:25
 */

public abstract class MoojNode {    
    // to hide: override and make return true.
    // XXX: replace with getDefaultMoojNodeView()
    // XXX: speaking of getDefaultMoojNodeView(), how about registering new nodes somewhere too?
    public boolean hideWhenNoChildren() {
        return false;
    }
    
    public JMenu getJMenu() {
        return null;
        //return new JMenu(this.getClass().getName());
    }
    
    public Object[] getChildren() {
        return new Object[0];
    }
    
    public Icon getIcon() {
        return null;
    }
    
    // call after updating children
    protected void childrenUpdatedAlert() {
        NodeManagerSingleton.getSingleton().childrenUpdated(this);
    }

    // call after this is renamed or icon changed
    protected void nodeChangedAlert() {
        NodeManagerSingleton.getSingleton().nodeChanged(this);
    }
}

import javax.swing.*;
/*
 * MoojNode.java
 *
 * 
 *
 * Created on 1 August 2002, 13:25
 */

public abstract class Resource {
    protected OpenFile openFile;
    
    public static final Object[] NO_CHILDREN = new Object[0];
    
    public Resource(OpenFile openFile){
        this.openFile = openFile;
    }
    
    public OpenFile getOpenFile() {
        return openFile;
    }
    
    // to hide: override and make return true.
    // XXX: replace with getDefaultResourceView()
    // XXX: speaking of getDefaultResourceView(), how about registering new nodes somewhere too?
    public boolean hideWhenNoChildren() {
        return false;
    }
    
    public JMenu getJMenu() {
        return null;
        //return new JMenu(this.getClass().getName());
    }
    
    public Object[] getChildren() {
        return NO_CHILDREN;
    }
    
    public Icon getIcon() {
        return null;
    }

    // how to respond to a rename event
    public void rename(String name) {
	//XXX
	return;
    }
    
    // default action when clicked (e.g in a list)
    public void clickAction() {
        return;
    }
    
    // default action when double-clicked (e.g in a list)
    public void doubleClickAction() {
        System.out.println(this.getClass() + " -- " + openFile + " -- children: " + getChildren().length);
    }
    /*
    // call after updating children
    protected void fireChildrenUpdated() {
        NodeManagerSingleton.getSingleton().childrenUpdated(this);
    }

    // call after this is renamed or icon changed
    protected void fireNodeChanged() {
        NodeManagerSingleton.getSingleton().nodeChanged(this);
    }
     */
}

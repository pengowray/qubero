package net.pengo.resource;
import net.pengo.app.*;

import javax.swing.*;
import java.awt.event.ActionEvent;
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
    // FIXME: replace with getDefaultResourceView()
    // FIXME: speaking of getDefaultResourceView(), how about registering new nodes somewhere too?
    public boolean hideWhenNoChildren() {
        return false;
    }
    
    public JMenu getJMenu() {
		JMenu menu = new JMenu("Default");
		Action defaultAction = new AbstractAction("Double click action") {
			public void actionPerformed(ActionEvent e) {
				Resource.this.doubleClickAction();
			}
		};
		menu.add(defaultAction);

		//Action action2 = new InfoAction();
			
		return menu;
			
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
	//FIXME:
	return;
    }
    
    // default action when clicked (e.g in a list)
    public void clickAction() {
        return;
    }
    
    // default action when double-clicked (e.g in a list)
    public void doubleClickAction() {
        System.out.println(this + "\n  " + this.getClass() + " -- children: " + getChildren().length);
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

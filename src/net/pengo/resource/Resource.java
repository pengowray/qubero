package net.pengo.resource;
import java.awt.event.ActionEvent;

import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.Icon;
import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.app.OpenFile;
/*
 * MoojNode.java
 *
 * What is a resource? An item in the menu? A definition? A declaration?
 * Something more?
 *
 * Created on 1 August 2002, 13:25
 */

public abstract class Resource {

    protected String name;
    
    public Resource(){
        super();
    }
    

    
    // to hide: override and make return true.
    // FIXME: replace with getDefaultResourceView()
    // FIXME: speaking of getDefaultResourceView(), how about registering new nodes somewhere too?
    public boolean hideWhenNoChildren() {
        return false;
    }
    
    public void giveActions(JMenu m) {
        //Insert these lines in subtypes:
        //super.giveActions(m);
        //m.add(new JSeparator());
        
        m.add(
                new AbstractAction("Double click action") {
                    public void actionPerformed(ActionEvent e) {
                        Resource.this.doubleClickAction();
                    }
                }
        );
        
        
    }
    
    public JMenu getJMenu() {
		JMenu menu = new JMenu("Default");
		giveActions(menu);
		return menu;
    }
    
	//xxx: put this back in
	//abstract public boolean isPrimative();
    
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
        System.out.println(this + "\n  " + this.getClass());
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
    
    

    public String getName() {
        return name;
    }

    /**
     * @param name The name to set.
     */
    public void setName(String name) {
        this.name = name;
    }
    
    public String toString() {
        String name;
        String pointer = "";
        if (isPointer()) {
            pointer = "(*)";
        }
        
        if (getName() == null) {
            name = getTypeName() + pointer + "=" + valueDesc();
        } else {
            name = "\"" + getName() + "\"" + pointer + "=" + valueDesc();
        }
        
        return name;
    }
    
    /** a description of the (evalutated) value stored by this resource */
    public String valueDesc() {
        return "none";
    }

    /** @return true if this a pointer or wrapper or reference or the like */ 
    public boolean isPointer() {
        return false;
    }
    
    /** The name of the class. this will be replaced with Qubero specific types eventually;.
     * @returns eg "IntResource" instead of "net.pengo.resource.IntResource" */
    public String getTypeName(){
        return shortTypeName(getClass());
    }    

    public static String shortTypeName(Class cl){
        String name = cl.getName();
        int dot = name.lastIndexOf(".");
        String shortName = name.substring(dot+1); 
        
        return shortName;
    }    
}

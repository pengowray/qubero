package net.pengo.resource;
import java.awt.event.ActionEvent;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import javax.swing.AbstractAction;
import javax.swing.Icon;
import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.propertyEditor.NamePage;
import net.pengo.propertyEditor.ResourceForm;

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
    private Set sinkSet = new HashSet();
    
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
        
        Resource[] res = getSources();
        if (res != null && res.length > 0) {
            JMenu resMenu = new JMenu("Resources");
            for (int i = 0; i < res.length; i++) {
                Resource node = (Resource)res[i];
                String name = node.getName();
                if (!node.isOwner(this))
                    name += " (used by " + node.getSinkCount() + ")";
                resMenu.add(node+"");
            }
            
            m.add(resMenu);
            
        }        
        m.add(new JSeparator());
        
        m.add(
                new AbstractAction("Property editor...") {
                    public void actionPerformed(ActionEvent e) {
                        editProperties();
                    }
                }
        );
        
    }
    
    /** @return list of PropertyPage's */ 
    public List getPrimaryPages() {
        List pp = new ArrayList();
        pp.add(new NamePage(this));
        return pp;
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
        
        return nameOrType() + pointer + "=" + valueDesc();
    }
    
    /** does not include a value */
    public String nameOrType(){
        if (getName() == null) {
            return getTypeName();
        } else {
            return "\"" + getName() + "\"";
        }
        
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

    
    // qnode things
    
    abstract public Resource[] getSources();

    
    public int getSinkCount(){
        return sinkSet.size();
    }
    
    public void addSink(Resource sink){
        sinkSet.add(sink);
    }
    
    public void removeSink(Resource sink){
        sinkSet.remove(sink);
    }
    
    //fixme: future versions will have a more complete assignment/conversion API
    public boolean isAssignableFrom(Class cl) {
        return (this.getClass().isAssignableFrom(cl));
    }
    
    //fixme: is name correct? is assignableTo really opposite to assignableFrom
    public boolean isAssignableTo(Class cl) {
        return (cl.isAssignableFrom(this.getClass()));
    }
    
    //for classes that act as direct references. getValue() will evalute the references.
    public boolean isReference() {
        return false;
    }
    
    //evalute, was getValue()
    public Resource evaluate() {
        return this;
    }
    
    public void editProperties() {
        new ResourceForm(this).show();
    }
    
    //quoted
    public Resource getPrimarySource() {
        return this;
    }
    
    // is this the owner? owners should edit values directly rather than selecting pointers from drop downs
    public boolean isOwner(Resource qnr) {
        List srcs = Arrays.asList(this.getSources());
        if (qnr.getSinkCount() == 1 &&
                (srcs.contains(qnr) 
                        //||  // or check if parent is owner maybe or something
                )) {
            return true;
        }
        
        //System.out.println("sinks: " + qnr.getSinkCount());
        return false;
    }    
    
    
    // listener stuff? etc etc
    
}

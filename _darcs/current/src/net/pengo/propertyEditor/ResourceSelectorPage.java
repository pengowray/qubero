/**
 * ResourceSelectorPage.java
 *
 * This is kinda a JavaPointer editor page
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import javax.swing.JLabel;
import java.util.List;
import javax.swing.JComboBox;

import net.pengo.pointer.JavaPointer;
import net.pengo.resource.Resource;
import net.pengo.pointer.ResourceRegistry;

public class ResourceSelectorPage extends EditablePage {
    JavaPointer jp;
    
    Resource initialValue;
    Resource[] values;
    JComboBox combo;

    Resource context; // the parent context.. for naming and checking ownership
    
    public ResourceSelectorPage(JavaPointer jp) {
        this(jp,null);
    }
    
    public ResourceSelectorPage(JavaPointer jp, PropertiesForm form) {
        super(form);
        this.jp = jp;
        
        //add(new JLabel( "Pointer name: "+ jp));
        //add(new JLabel( "Points to: "));
        
        values = values();
        combo = new JComboBox(values);
        combo.addActionListener( getModActionListener() );
        add(combo);
    }
    
    private Resource[] values() {
        //fixme: might need a "none of the above" or "reset to default" selection
        //       and a way to inform the user of this form about it.
        //       guess the Form should accept a new object for this task for extra feedback.
        //       maybe the form could just be a listener.
        // 	 main thing needed to be rememebered is the old value so it can go back with an undo.
        //       or a "default" value for when use-a-pointer to switched off
        
        //first (and selected) option is always what jp was set to initially:
        //initialValue = jp.evalute();
        initialValue = jp.getPrimarySource();
        
        //fixme: have a "primative" option too? maybe this will be elsewhere
        //primativeValue = ...?;
        
        List l = ResourceRegistry.instance().getAllOfType( jp.getType() );
        //List l = ResourceRegistry.instance().getAll(); //OfType( jp.getType() );
        l.add(0,initialValue);
        
        return (Resource[])l.toArray(new Resource[0]);
    }
    
    public void buildOp() {
        //combo.setSelectedIndex(  );
    }
    
    public void saveOp() {
        int selected = combo.getSelectedIndex();
        jp.setValue(values[selected]);
        //res.setSigned( type.getSelectedIndex() );
        //System.out.println("setting sign to: " + type.getSelectedIndex());
    }
    
    public String toString() {
        return jp.nameOrType();
    }
    
    public Resource getContext() {
        return context;
    }

    public void setContext(Resource context) {
        this.context = context;
    }    
    
}


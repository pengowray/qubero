/*
 * DefaultDefinitionResource.java
 
 DefaultDefinitionResource is an "untyped" resource. (or not yet typed).
 It is based on a selection.
 
 XXX: Perhaps this should be mereged with DefaultSelectionResource as
 this class is converted to that one when it is used.
 
 *
 * Created on 21 August 2002, 18:18
 */

package net.pengo.resource;

import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.pointer.JavaPointer;

/**
 * This one's going the way of the dodo
 *
 * @author  peter halasz
 */
public class DefaultDefinitionResource extends DefinitionResource implements AddressedResource
{
    private int sinkCount;
    
    public final JavaPointer selResP = new JavaPointer("net.pengo.resource.SelectionResource"); //private SelectionResource selRes;
    
    //final protected LongListSelectionModel sel;
    
    public Resource[] getSources() {
        return new Resource[]{};
    }
    
    public void editProperties(){};
    
    public DefaultDefinitionResource(SelectionResource sel) {
        super();
        selResP.setValue(sel);
    }

    public void giveActions(JMenu menu) {
        super.giveActions(menu);
        menu.add(new JSeparator());

        TypeRegistry.instance().giveConversionActions(menu, getSelectionResource());
        //menu.add(new JSeparator());
    }
    
    public String valueDesc() {
        return selResP.evaluate().valueDesc();
    }    
    
    public void doubleClickAction() {
        // duplicated in BooleanAddrssdResource
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
    
    
    
    public net.pengo.restree.ResourceList getSubResources() {
        return null;
    }
    
    
    
    public void removeSink(Resource sink) {
        sinkCount--;
    }
    
    public int getSinkCount() {
        return sinkCount;
    }
    
    public void addSink(Resource sink) {
        sinkCount++;
    }

    public SelectionResource getSelectionResource() {
        return (SelectionResource)selResP.evaluate();
    }
    
    public void setSelectionResource(SelectionResource selRes) {
        selResP.setValue(selRes);
    }
    
    
}

package net.pengo.resource;

import javax.swing.Action;

import net.pengo.pointer.ResourceRegistry;

/*
 *
 */

public abstract class DefinitionResource extends Resource //implements DependencyListener
{

    //Subresources must be definition resources?
    //i.e. part of the "state" ?
    //abstract public ResourceList getSubResources();
    
    public DefinitionResource() {
        super();
        ResourceRegistry.instance().add(this);
    }
    
    
    
    
    
    
    
    
}

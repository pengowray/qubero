/*
 * Created on Jan 16, 2004
 */
package net.pengo.resource;

import net.pengo.pointer.QFunction;

/**
 * @author Smiley
 *
 * */
public abstract class TypeResource extends Resource {

    public TypeResource() {
        super();
        // TODO Auto-generated constructor stub
    }

    public Resource[] getSources() {
        // TODO Auto-generated method stub
        //return null;
		return new Resource[0];
    }

    public abstract boolean canConvertTo(TypeResource type, Aspect aspect);
    public abstract boolean canConvertTo(TypeResource x);
    
    // can convert there and back again, without loss in "aspect".
    public abstract boolean canIsomorphicConvert(TypeResource x, Aspect aspect);
    
    public abstract QFunction[] getFunctions();
    
    public abstract QFunction[] getConstructors();
    abstract public QFunction[] getConstructorsForSelections();
    
    //isDynamic() ?
    
    public abstract boolean equals(TypeResource tr);


}

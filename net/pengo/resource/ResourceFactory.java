/**
 * ResourceFactory.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;
import java.util.List;
import net.pengo.app.OpenFile;
import net.pengo.selection.LongListSelectionModel;

public class ResourceFactory
{
	public static Resource wrap(Object o, OpenFile openFile) {
		if (o instanceof LongListSelectionModel)
			return wrap((LongListSelectionModel)o, openFile);
		
                if (o instanceof List)
			return wrap((List)o, openFile);

		if (o instanceof Resource)
			return wrap((Resource)o, openFile);
		
		return new ContainerResource(o, openFile);
	}
	
	public static Resource wrap(List o, OpenFile openFile) {
		return new ListResource(o, openFile);
	}

	// dont wrap (already wrapped) resources
	public static Resource wrap(Resource o, OpenFile openFile) {
		//fixme: maybe check that o belongs to openFile ?
		return o;
	}
	
	public static Resource wrap(LongListSelectionModel o, OpenFile openFile) {
		return new DefaultSelectionResource(openFile, o);
	}
	
	/*
	public static TreeResource wrap(List c, OpenFile openFile) {
		
	}
	 */
}

